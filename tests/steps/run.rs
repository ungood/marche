use std::time::{Duration, Instant};

use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use serde_json::Value;

use super::ows::contains;
use super::services::Services;
use super::world::{MarcheWorld, docstring, marche, output_of, output_within, plain};

/// Subcommands that talk to Temporal, so `I run` points them at the test server.
const TEMPORAL_COMMANDS: &[&str] = &["run", "show", "status", "list", "cancel", "worker"];

#[given(regex = r#"^the file "([^"]+)":$"#)]
fn file(world: &mut MarcheWorld, path: String, step: &Step) {
    world.write(&path, &format!("{}\n", docstring(step)));
}

#[given(regex = r#"^the environment variable "([^"]+)" is "([^"]*)"$"#)]
fn environment_variable(world: &mut MarcheWorld, name: String, value: String) {
    world.env.push((name, value));
}

#[given("Temporal's configuration file says:")]
fn temporal_config(world: &mut MarcheWorld, step: &Step) {
    std::fs::write(world.temporal_config(), format!("{}\n", docstring(step)))
        .expect("write temporal.toml");
}

#[given(regex = r#"^a worker started with "(.+)"$"#)]
async fn worker_started_with(world: &mut MarcheWorld, command: String) {
    let words = shell_words::split(&command).expect("the command splits into words");
    let args = words
        .strip_prefix(&["marche".to_string(), "worker".to_string()])
        .expect("the command starts with marche worker");
    world.services = Some(Services::start_with(args.to_vec()).await);
}

#[given("a Temporal server with no worker")]
async fn server_without_worker(world: &mut MarcheWorld) {
    world.services = Some(Services::start_without_worker().await);
}

#[given("the worker has stopped")]
fn worker_stopped(world: &mut MarcheWorld) {
    world
        .services
        .as_mut()
        .expect("the scenario has no worker of its own")
        .stop_worker();
}

#[when(regex = r#"^I run "(.+)"$"#)]
async fn i_run(world: &mut MarcheWorld, command: String) {
    let mut run = command_for(world, &command).await;
    world.last = Some(output_of(&mut run).await);
}

/// Runs a command that may not end by itself, and stops it with SIGTERM if
/// it's still running after that many seconds.
#[when(regex = r#"^I run "(.+)" for (\d+) seconds?$"#)]
async fn i_run_for(world: &mut MarcheWorld, command: String, seconds: u64) {
    let mut run = command_for(world, &command).await;
    let (output, stopped) = output_within(&mut run, Duration::from_secs(seconds)).await;
    world.last = Some(output);
    world.still_running = stopped;
}

/// marche with the scenario's environment, ready to run in its directory.
async fn command_for(world: &MarcheWorld, command: &str) -> tokio::process::Command {
    let words = shell_words::split(&world.expand(command)).expect("the command splits into words");
    let (program, args) = words.split_first().expect("the command isn't empty");
    assert_eq!(program, "marche", "commands start with marche");
    let mut run = tokio::process::Command::new(marche());
    // Temporal's settings come only from the scenario.
    for (name, _) in std::env::vars().filter(|(name, _)| name.starts_with("TEMPORAL_")) {
        run.env_remove(name);
    }
    run.args(args)
        .current_dir(world.work_dir())
        .env("NO_COLOR", "1")
        .env("XDG_STATE_HOME", world.state_dir())
        .env("TEMPORAL_CONFIG_FILE", world.temporal_config());
    if args
        .first()
        .is_some_and(|command| TEMPORAL_COMMANDS.contains(&command.as_str()))
    {
        run.env("TEMPORAL_ADDRESS", world.address().await);
    }
    run.envs(world.env.iter().map(|(name, value)| (name, value)));
    run
}

fn describe(world: &MarcheWorld) -> String {
    let output = world.last();
    format!(
        "exit status: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[then("the command succeeds")]
fn succeeds(world: &mut MarcheWorld) {
    assert!(
        world.last().status.success(),
        "marche failed\n{}",
        describe(world)
    );
}

#[then("the command is still running")]
fn still_running(world: &mut MarcheWorld) {
    assert!(world.still_running, "marche had ended\n{}", describe(world));
}

#[then("the command fails")]
fn fails(world: &mut MarcheWorld) {
    assert!(
        !world.last().status.success(),
        "marche succeeded\n{}",
        describe(world)
    );
}

#[then(expr = "the command fails with exit code {int}")]
fn fails_with_code(world: &mut MarcheWorld, code: i32) {
    assert_eq!(
        world.last().status.code(),
        Some(code),
        "wrong exit code\n{}",
        describe(world)
    );
}

/// The doc string's lines, without blank ones, with `$RUN` and `$ULID`
/// replaced.
fn lines(world: &MarcheWorld, step: &Step) -> Vec<String> {
    world
        .expand(&docstring(step))
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| plain(line.as_bytes()))
        .collect()
}

/// Checks that every line of the doc string appears in the text.
fn says(world: &MarcheWorld, actual: &[u8], step: &Step, what: &str) {
    let actual = plain(actual);
    for line in lines(world, step) {
        assert!(
            actual.contains(&line),
            "{what} doesn't say {line:?}:\n{actual}"
        );
    }
}

/// Checks that the doc string's lines appear in the text in their order, each
/// after the one before.
fn says_in_order(world: &MarcheWorld, actual: &[u8], step: &Step, what: &str) {
    let actual = plain(actual);
    let mut rest = actual.as_str();
    for line in lines(world, step) {
        let Some(found) = rest.find(&line) else {
            panic!("{what} doesn't say {line:?} after the lines before it:\n{actual}");
        };
        rest = &rest[found + line.len()..];
    }
}

#[then("the output says:")]
fn output_says(world: &mut MarcheWorld, step: &Step) {
    says(world, &world.last().stdout, step, "the output");
}

#[then("the output says, in order:")]
fn output_says_in_order(world: &mut MarcheWorld, step: &Step) {
    says_in_order(world, &world.last().stdout, step, "the output");
}

/// The version `git describe` gives this checkout, without the tag's `v`, or
/// Cargo.toml's placeholder when there's no release tag to describe.
pub async fn marche_version() -> String {
    let described = tokio::process::Command::new("git")
        .args(["describe", "--tags", "--match", "v[0-9]*"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .await
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
    described
        .as_deref()
        .and_then(|tag| tag.strip_prefix('v'))
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_string()
}

#[then("the output is marche's version")]
async fn output_is_version(world: &mut MarcheWorld) {
    let version = marche_version().await;
    let output = String::from_utf8_lossy(&world.last().stdout);
    assert_eq!(output.trim(), format!("marche {version}"));
}

#[then("the error says:")]
fn error_says(world: &mut MarcheWorld, step: &Step) {
    says(world, &world.last().stderr, step, "the error");
}

#[then("the help says:")]
fn help_says(world: &mut MarcheWorld, step: &Step) {
    let stderr = String::from_utf8_lossy(&world.last().stderr).into_owned();
    let (_, help) = stderr
        .split_once("help:")
        .unwrap_or_else(|| panic!("the error has no help\n{}", describe(world)));
    says(world, help.as_bytes(), step, "the help");
}

#[then(regex = r#"^the error points at line (\d+) of "([^"]+)"$"#)]
fn points_at(world: &mut MarcheWorld, line: usize, file: String) {
    // miette heads each snippet with [<file>:<line>:<column>].
    let snippet = format!("[{file}:{line}:");
    let stderr = plain(&world.last().stderr);
    assert!(
        stderr.contains(&snippet),
        "the error doesn't point at line {line} of {file}:\n{stderr}"
    );
}

#[then("marche prints:")]
fn prints(world: &mut MarcheWorld, step: &Step) {
    let output = world.last();
    let both = [output.stderr.as_slice(), output.stdout.as_slice()].concat();
    says(world, &both, step, "marche");
}

/// Standard error comes first, so task lines read in order before the output.
#[then("marche prints, in order:")]
fn prints_in_order(world: &mut MarcheWorld, step: &Step) {
    let output = world.last();
    let both = [output.stderr.as_slice(), output.stdout.as_slice()].concat();
    says_in_order(world, &both, step, "marche");
}

#[then("marche doesn't print:")]
fn doesnt_print(world: &mut MarcheWorld, step: &Step) {
    let output = world.last();
    let both = plain(&[output.stderr.as_slice(), output.stdout.as_slice()].concat());
    for line in lines(world, step) {
        assert!(!both.contains(&line), "marche prints {line:?}:\n{both}");
    }
}

#[then("the worker prints:")]
async fn worker_prints(world: &mut MarcheWorld, step: &Step) {
    let services = world
        .services
        .as_ref()
        .expect("the scenario has no worker of its own");
    // The worker's output reaches the log through a pipe, a moment later.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let log = services.log.lock().expect("worker log lock").clone();
        let log = plain(log.as_bytes());
        let missing = lines(world, step)
            .into_iter()
            .find(|line| !log.contains(line));
        match missing {
            None => return,
            Some(line) if Instant::now() >= deadline => {
                panic!("the worker doesn't print {line:?}:\n{log}")
            }
            Some(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

#[then("the JSON output contains:")]
fn json_contains(world: &mut MarcheWorld, step: &Step) {
    let actual: Value = serde_json::from_slice(&world.last().stdout).expect("the output is JSON");
    let expected: Value =
        serde_json::from_str(&world.expand(&docstring(step))).expect("the doc string is JSON");
    assert!(
        contains(&actual, &expected),
        "{actual:#} doesn't contain {expected:#}"
    );
}
