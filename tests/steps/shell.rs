//! Steps for shell steps' processes, logs and cancellation, and for a worker
//! lost during a step. Steps that wait are async, so scenarios running beside
//! them carry on.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use serde_json::Value;
use tokio::process::Command;

use super::ows::{finish, task};
use super::world::MarcheWorld;

/// How long a step has to do what a scenario waits for.
const PATIENCE: Duration = Duration::from_secs(60);

/// Polls until `done` holds, or panics with `what` after `limit`.
async fn wait_until(limit: Duration, what: &str, mut done: impl AsyncFnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !done().await {
        assert!(Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Waits for the run started by `the workflow is running its <task> task`.
pub async fn wait_for_run(world: &mut MarcheWorld) {
    let pending = world.pending.take().expect("the workflow isn't running");
    let output = tokio::task::spawn_blocking(move || pending.wait_with_output())
        .await
        .expect("wait for marche run")
        .expect("wait for marche run");
    let result = serde_json::from_slice(&output.stdout).expect("marche run --json prints JSON");
    finish(world, result);
}

#[given(regex = r#"^the workflow has written the file "([^"]+)"$"#)]
async fn written(world: &mut MarcheWorld, path: String) {
    let path = world.work_dir().join(path);
    let what = format!("{} wasn't written", path.display());
    wait_until(PATIENCE, &what, async || path.exists()).await;
}

#[when(regex = r"^the worker is killed and started again during each of (\d+) attempts$")]
async fn lose_worker(world: &mut MarcheWorld, attempts: u32) {
    let services = world.services.as_mut().expect("the workflow isn't running");
    let runs = services.state.join("marche/runs");
    for attempt in 1..=attempts {
        // The activity creates the attempt's log before it starts the step.
        let log = format!("{attempt}.stdout");
        let what = format!("attempt {attempt} didn't start");
        wait_until(PATIENCE, &what, async || find(&runs, &log)).await;
        services.restart_worker().await;
    }
    wait_for_run(world).await;
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

/// Runs the Temporal CLI against the scenario's server, and returns its output.
async fn temporal(address: &str, args: &[&str]) -> Vec<u8> {
    let output = Command::new("temporal")
        .args(args)
        .args(["--address", address])
        .output()
        .await
        .expect("run the temporal CLI");
    assert!(
        output.status.success(),
        "temporal {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[when("the workflow is cancelled")]
async fn cancel(world: &mut MarcheWorld) {
    let address = world.address();
    // The scenario's own server runs only this workflow.
    let list = [
        "workflow",
        "list",
        "--query",
        "ExecutionStatus = 'Running'",
        "--output",
        "json",
    ];
    let mut id = None;
    wait_until(PATIENCE, "the workflow isn't listed", async || {
        let listed = temporal(&address, &list).await;
        let listed: Value = serde_json::from_slice(&listed).unwrap_or(Value::Null);
        id = listed
            .as_array()
            .and_then(|runs| runs.first())
            .and_then(|run| run["execution"]["workflowId"].as_str())
            .map(str::to_string);
        id.is_some()
    })
    .await;
    let id = id.expect("the workflow's ID");
    temporal(&address, &["workflow", "cancel", "--workflow-id", &id]).await;
    wait_for_run(world).await;
}

#[then(regex = r#"^the process whose ID is in "([^"]+)" has stopped$"#)]
async fn stopped(world: &mut MarcheWorld, path: String) {
    let path: PathBuf = world.work_dir().join(path);
    let pid: libc::pid_t = std::fs::read_to_string(&path)
        .expect("read the process ID")
        .trim()
        .parse()
        .expect("the file holds a process ID");
    // An exited process can linger briefly until its new parent reaps it.
    let what = format!("process {pid} is still running");
    // SAFETY: signal 0 only checks that the process exists.
    wait_until(Duration::from_secs(2), &what, async || unsafe {
        libc::kill(pid, 0) != 0
    })
    .await;
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
