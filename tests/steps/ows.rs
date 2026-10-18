//! Steps for the Open Workflow conformance kit, as its README defines them,
//! plus the filter that picks which of its scenarios marche runs.
//!
//! The harness drives the real binary: `marche run --json` executes the
//! workflow and waits for it, and `marche show --json` lists its tasks in the
//! order they started.

use std::process::Stdio;

use cucumber::gherkin::{Scenario, Step};
use cucumber::{given, then, when};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use super::services::Services;
use super::world::{MarcheWorld, Pending, docstring, marche};
use super::{inspect, shell};

/// Task types the interpreter supports. A conformance scenario runs only when
/// every task in its definition is listed here, so each task type's work adds
/// its name when it lands.
const SUPPORTED: &[&str] = &[
    "do",
    "set",
    "run: shell",
    "switch",
    "raise",
    "try",
    "wait",
    "fork",
    "for",
];

/// `do` comes last, because `for` tasks have a `do` too.
const TASK_TYPES: &[&str] = &[
    "call", "emit", "for", "fork", "listen", "raise", "run", "set", "switch", "try", "wait", "do",
];

/// The processes a `run` task can run. Its other keys, such as `return` and
/// `await`, are settings.
const PROCESSES: &[&str] = &["shell", "script", "container", "workflow"];

/// Whether marche runs this conformance kit scenario. Scenarios without a
/// workflow definition always run.
pub fn is_supported(scenario: &Scenario) -> bool {
    unsupported(scenario).is_empty()
}

/// The task types in the scenario's workflow definition that aren't in
/// `SUPPORTED`.
pub fn unsupported(scenario: &Scenario) -> Vec<String> {
    let Some(step) = scenario
        .steps
        .iter()
        .find(|step| step.value == "a workflow with definition:")
    else {
        return Vec::new();
    };
    let definition: Value = serde_yaml_ng::from_str(&docstring(step)).expect("definition is YAML");
    let mut types = Vec::new();
    task_types(&definition["do"], &mut types);
    types.retain(|kind| !SUPPORTED.contains(&kind.as_str()));
    types
}

/// Collects the type of every task in a task list, recursing into nested lists.
/// `run` tasks are named by process, as in `run: shell`.
fn task_types(list: &Value, types: &mut Vec<String>) {
    let Some(items) = list.as_array() else { return };
    for task in items
        .iter()
        .filter_map(Value::as_object)
        .flat_map(|item| item.values())
    {
        if let Some(kind) = TASK_TYPES.iter().find(|kind| task.get(**kind).is_some()) {
            match (*kind, task["run"].as_object()) {
                ("run", Some(run)) => {
                    let processes = PROCESSES
                        .iter()
                        .filter(|process| run.contains_key(**process));
                    types.extend(processes.map(|process| format!("run: {process}")));
                }
                _ => types.push(kind.to_string()),
            }
        }
        task_types(&task["do"], types);
        task_types(&task["try"], types);
        task_types(&task["catch"]["do"], types);
        task_types(&task["fork"]["branches"], types);
    }
}

pub fn yaml(step: &Step) -> Value {
    serde_yaml_ng::from_str(&docstring(step)).expect("doc string is YAML")
}

#[given("a workflow with definition:")]
fn definition(world: &mut MarcheWorld, step: &Step) {
    world.definition = Some(docstring(step));
}

#[given("given the workflow input is:")]
fn input(world: &mut MarcheWorld, step: &Step) {
    world.input = Some(yaml(step));
}

/// `marche run --json` for the scenario's definition and input.
async fn run_command(world: &MarcheWorld) -> Command {
    let path = world.write(
        "workflow.yaml",
        world.definition.as_deref().expect("no workflow definition"),
    );
    let mut run = Command::new(marche());
    run.arg("run")
        .arg(&path)
        .args(["--json", "--address", &world.address().await])
        .current_dir(world.work_dir())
        .env("NO_COLOR", "1")
        .env("XDG_STATE_HOME", world.state_dir());
    if let Some(input) = &world.input {
        run.args(["--input", &input.to_string()]);
    }
    run
}

/// Records a finished run, and its tasks from `marche show --json`.
pub async fn finish(world: &mut MarcheWorld, result: Value) {
    let id = result["run"]
        .as_str()
        .expect("marche run --json has a run ID")
        .to_string();
    let show = json(
        Command::new(marche())
            .args(["show", &id, "--json", "--address", &world.address().await])
            .env("NO_COLOR", "1")
            .env("XDG_STATE_HOME", world.state_dir()),
    )
    .await;
    world.run_id = Some(id);
    world.run = Some(result);
    world.show = Some(show);
}

#[when("the workflow is executed")]
async fn execute(world: &mut MarcheWorld) {
    let started = std::time::Instant::now();
    let result = json(&mut run_command(world).await).await;
    world.took = Some(started.elapsed());
    finish(world, result).await;
}

#[given(regex = r"^the workflow is running its (\w+) task$")]
async fn running(world: &mut MarcheWorld, task: String) {
    world.services = Some(Services::start().await);
    let mut child = run_command(world)
        .await
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("start marche run");
    let stderr = child.stderr.take().expect("run stderr");
    // marche run prints "started <reference>" as each task starts.
    let suffix = format!("/{task}");
    let (started, has_started) = tokio::sync::oneshot::channel();
    let stderr = tokio::spawn(async move {
        let mut started = Some(started);
        let mut text = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.starts_with("started ") && line.ends_with(&suffix) {
                started.take().map(|started| started.send(()));
            }
            text.push_str(&line);
            text.push('\n');
        }
        text
    });
    world.pending = Some(Pending { child, stderr });
    assert!(has_started.await.is_ok(), "{task} didn't start");
    let address = world.address().await;
    world.run_id = Some(inspect::run_started_in(&address, &world.work_dir()).await);
}

/// Uses the worker's hidden test hook: it continues each run as new once that
/// many tasks have finished in it, as if Temporal had suggested it.
#[given(regex = r"^the worker continues runs as new after every (\d+) tasks$")]
async fn continues_as_new(world: &mut MarcheWorld, tasks: String) {
    world.services =
        Some(Services::start_with(vec!["--continue-as-new-after".into(), tasks]).await);
}

#[when("the worker is killed and started again")]
async fn restart_worker(world: &mut MarcheWorld) {
    world
        .services
        .as_mut()
        .expect("the workflow isn't running")
        .restart_worker()
        .await;
    shell::wait_for_run(world).await;
}

/// Runs marche and parses its standard output, whatever the exit code, since a
/// faulted workflow still reports its result.
async fn json(command: &mut Command) -> Value {
    let output = command.output().await.expect("run marche");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "marche didn't print JSON ({error}):\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn run(world: &MarcheWorld) -> &Value {
    world.run.as_ref().expect("the workflow wasn't executed")
}

fn assert_status(actual: &Value, expected: &str, what: &str) {
    assert_eq!(
        actual["status"].as_str(),
        Some(expected),
        "{what} didn't {expected}: {actual:#}"
    );
}

/// Whether every key in `expected` appears in `actual` with a matching value,
/// and arrays match item by item. Runtimes may add detail, such as an error's
/// `detail`.
pub fn contains(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => expected
            .iter()
            .all(|(key, value)| actual.get(key).is_some_and(|a| contains(a, value))),
        (Value::Array(actual), Value::Array(expected)) => {
            actual.len() == expected.len()
                && actual.iter().zip(expected).all(|(a, e)| contains(a, e))
        }
        _ => actual == expected,
    }
}

fn property<'a>(output: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(output, |value, key| value.get(key))
}

#[then("the workflow should complete")]
fn completes(world: &mut MarcheWorld) {
    assert_status(run(world), "completed", "the workflow");
}

#[then("the workflow should complete with output:")]
fn completes_with(world: &mut MarcheWorld, step: &Step) {
    let run = run(world);
    assert_status(run, "completed", "the workflow");
    assert_eq!(run["output"], yaml(step), "workflow output");
}

#[then("the workflow should fault")]
fn faults(world: &mut MarcheWorld) {
    assert_status(run(world), "faulted", "the workflow");
}

#[then("the workflow should fault with error:")]
fn faults_with(world: &mut MarcheWorld, step: &Step) {
    let run = run(world);
    assert_status(run, "faulted", "the workflow");
    let expected = yaml(step);
    assert!(
        contains(&run["error"], &expected),
        "error {:#} doesn't match {expected:#}",
        run["error"]
    );
}

#[then("the workflow should cancel")]
fn cancels(world: &mut MarcheWorld) {
    assert_status(run(world), "cancelled", "the workflow");
}

#[then(regex = r"^the workflow output should have properties (.+)$")]
fn has_properties(world: &mut MarcheWorld, list: String) {
    let output = &run(world)["output"];
    for path in list.split(',').map(|path| path.trim().trim_matches('\'')) {
        assert!(
            property(output, path).is_some(),
            "output has no {path}: {output:#}"
        );
    }
}

#[then(regex = r"^the workflow output should have a '([^']+)' property with value:$")]
fn has_value(world: &mut MarcheWorld, path: String, step: &Step) {
    let output = &run(world)["output"];
    assert_eq!(
        property(output, &path),
        Some(&yaml(step)),
        "{path} in {output:#}"
    );
}

#[then(regex = r"^the workflow output should have a '([^']+)' property containing (\d+) items$")]
fn has_items(world: &mut MarcheWorld, path: String, count: usize) {
    let output = &run(world)["output"];
    let items = property(output, &path).and_then(Value::as_array);
    assert_eq!(items.map(Vec::len), Some(count), "{path} in {output:#}");
}

/// The tasks named `name`, in the order they started.
pub fn tasks(world: &MarcheWorld) -> Vec<&Value> {
    world.show.as_ref().expect("the workflow wasn't executed")["tasks"]
        .as_array()
        .expect("marche show --json has tasks")
        .iter()
        .collect()
}

fn position(world: &MarcheWorld, name: &str) -> usize {
    tasks(world)
        .iter()
        .position(|task| task["name"] == name)
        .unwrap_or_else(|| panic!("{name} didn't run"))
}

pub fn task<'a>(world: &'a MarcheWorld, name: &str) -> &'a Value {
    tasks(world)[position(world, name)]
}

#[then(regex = r"^(\w+) should run first$")]
fn runs_first(world: &mut MarcheWorld, name: String) {
    assert_eq!(position(world, &name), 0, "{name} didn't run first");
}

#[then(regex = r"^(\w+) should run last$")]
fn runs_last(world: &mut MarcheWorld, name: String) {
    let last = tasks(world).len() - 1;
    assert_eq!(position(world, &name), last, "{name} didn't run last");
}

#[then(regex = r"^(\w+) should run before (\w+)$")]
fn runs_before(world: &mut MarcheWorld, first: String, second: String) {
    assert!(
        position(world, &first) < position(world, &second),
        "{first} didn't run before {second}"
    );
}

#[then(regex = r"^(\w+) should run after (\w+)$")]
fn runs_after(world: &mut MarcheWorld, second: String, first: String) {
    assert!(
        position(world, &first) < position(world, &second),
        "{second} didn't run after {first}"
    );
}

#[then(regex = r"^(\w+) should run (\d+) times?$")]
fn runs_times(world: &mut MarcheWorld, name: String, count: usize) {
    let runs = tasks(world)
        .iter()
        .filter(|task| task["name"] == name)
        .count();
    assert_eq!(runs, count, "{name} ran {runs} times");
}

#[then(regex = r"^the workflow should have continued as new (\d+) times?$")]
async fn continued_as_new(world: &mut MarcheWorld, count: usize) {
    let id = run(world)["run"].as_str().expect("the run has an ID");
    let address = world.address().await;
    // Every run in the chain has the workflow's ID, though visibility can take
    // a moment to list the last one.
    let mut runs = 0;
    for _ in 0..50 {
        let output = Command::new("temporal")
            .args(["workflow", "list", "--address", &address, "-o", "json"])
            .args(["--query", &format!("WorkflowId = '{id}'")])
            .output()
            .await
            .expect("run temporal workflow list");
        let list: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
        runs = list.as_array().map_or(0, Vec::len);
        if runs > count {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(runs, count + 1, "the workflow had {runs} Temporal runs");
}

#[then(regex = r"^(\w+) should (complete|fault|cancel)$")]
fn task_status(world: &mut MarcheWorld, name: String, status: String) {
    let expected = match status.as_str() {
        "complete" => "completed",
        "fault" => "faulted",
        _ => "cancelled",
    };
    assert_status(task(world, &name), expected, &name);
}

#[then(regex = r"^(\w+) should complete with output:$")]
fn task_output(world: &mut MarcheWorld, name: String, step: &Step) {
    let task = task(world, &name);
    assert_status(task, "completed", &name);
    assert_eq!(task["output"], yaml(step), "output of {name}");
}

#[then(regex = r"^(\w+) should fault with error:$")]
fn task_error(world: &mut MarcheWorld, name: String, step: &Step) {
    let task = task(world, &name);
    assert_status(task, "faulted", &name);
    let expected = yaml(step);
    assert!(
        contains(&task["error"], &expected),
        "error of {name} {:#} doesn't match {expected:#}",
        task["error"]
    );
}
