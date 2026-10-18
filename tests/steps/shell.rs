//! Steps for shell steps' processes, logs and cancellation, and for a worker
//! lost during a step.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use serde_json::Value;

use super::ows::{finish, task};
use super::world::MarcheWorld;

/// How long a step has to do what a scenario waits for.
const PATIENCE: Duration = Duration::from_secs(60);

/// Polls until `done` holds, or panics with `what` after `limit`.
fn wait_until(limit: Duration, what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Waits for the run started by `the workflow is running its <task> task`.
fn wait_for_run(world: &mut MarcheWorld) {
    let output = world
        .pending
        .take()
        .expect("the workflow isn't running")
        .wait_with_output()
        .expect("wait for marche run");
    let result = serde_json::from_slice(&output.stdout).expect("marche run --json prints JSON");
    finish(world, result);
}

#[given(regex = r#"^the workflow has written the file "([^"]+)"$"#)]
fn written(world: &mut MarcheWorld, path: String) {
    let path = world.work_dir().join(path);
    wait_until(
        PATIENCE,
        &format!("{} wasn't written", path.display()),
        || path.exists(),
    );
}

#[when(regex = r"^the worker is killed and started again during each of (\d+) attempts$")]
fn lose_worker(world: &mut MarcheWorld, attempts: u32) {
    let services = world.services.as_mut().expect("the workflow isn't running");
    let runs = services.state.join("marche/runs");
    for attempt in 1..=attempts {
        // The activity creates the attempt's log before it starts the step.
        let log = format!("{attempt}.stdout");
        wait_until(PATIENCE, &format!("attempt {attempt} didn't start"), || {
            find(&runs, &log)
        });
        services.restart_worker();
    }
    wait_for_run(world);
}

/// Whether a file with this name is anywhere under `dir`.
fn find(dir: &Path, name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            find(&path, name)
        } else {
            entry.file_name() == name
        }
    })
}

#[when("the workflow is cancelled")]
fn cancel(world: &mut MarcheWorld) {
    let address = world.address();
    let temporal = |args: &[&str]| {
        let output = Command::new("temporal")
            .args(args)
            .args(["--address", &address])
            .output()
            .expect("run the temporal CLI");
        assert!(
            output.status.success(),
            "temporal {args:?} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    // The scenario's own server runs only this workflow.
    let mut id = None;
    wait_until(PATIENCE, "the workflow isn't listed", || {
        let listed = temporal(&[
            "workflow",
            "list",
            "--query",
            "ExecutionStatus = 'Running'",
            "--output",
            "json",
        ]);
        let listed: Value = serde_json::from_slice(&listed).unwrap_or(Value::Null);
        id = listed
            .as_array()
            .and_then(|runs| runs.first())
            .and_then(|run| run["execution"]["workflowId"].as_str())
            .map(str::to_string);
        id.is_some()
    });
    let id = id.expect("the workflow's ID");
    temporal(&["workflow", "cancel", "--workflow-id", &id]);
    wait_for_run(world);
}

#[then(regex = r#"^the process whose ID is in "([^"]+)" has stopped$"#)]
fn stopped(world: &mut MarcheWorld, path: String) {
    let path: PathBuf = world.work_dir().join(path);
    let pid: libc::pid_t = std::fs::read_to_string(&path)
        .expect("read the process ID")
        .trim()
        .parse()
        .expect("the file holds a process ID");
    // An exited process can linger briefly until its new parent reaps it.
    wait_until(
        Duration::from_secs(2),
        &format!("process {pid} is still running"),
        // SAFETY: signal 0 only checks that the process exists.
        || unsafe { libc::kill(pid, 0) } != 0,
    );
}

#[then(regex = r"^(\w+) should have a standard output log of (\d+) bytes$")]
fn log_size(world: &mut MarcheWorld, name: String, bytes: u64) {
    let log = task(world, &name)["log"]
        .as_str()
        .unwrap_or_else(|| panic!("{name} has no log"));
    let size = std::fs::metadata(log)
        .unwrap_or_else(|error| panic!("read {log}: {error}"))
        .len();
    assert_eq!(size, bytes, "the size of {log}");
}
