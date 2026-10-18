//! Steps that start runs for `marche status`, `list`, `show` and `cancel` to
//! report on. They wait for Temporal's visibility, which `marche list` and
//! `marche status` read, because it can lag a moment behind a run.

use std::path::Path;

use cucumber::given;
use serde_json::Value;
use tokio::process::Command;

use super::shell::{PATIENCE, wait_until};
use super::world::{MarcheWorld, marche};

/// marche, to run from the directory that holds `path` in the scenario, and
/// the file's name.
async fn marche_beside(world: &MarcheWorld, path: &str) -> (Command, String) {
    let file = world.work_dir().join(path);
    let dir = file.parent().expect("the file has a directory");
    let name = file
        .file_name()
        .expect("the path names a file")
        .to_string_lossy()
        .into_owned();
    let mut command = Command::new(marche());
    command
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("XDG_STATE_HOME", world.state_dir())
        .env("TEMPORAL_CONFIG_FILE", world.temporal_config())
        .env("TEMPORAL_ADDRESS", world.address().await);
    (command, name)
}

/// Waits until visibility lists a run that matches the query.
async fn wait_listed(address: &str, query: &str) {
    let what = format!("Temporal doesn't list a run where {query}");
    wait_until(PATIENCE, &what, async || {
        !list(address, query).await.is_empty()
    })
    .await;
}

/// The ID of the run started in `directory` that's still running, by the
/// directory marche records on every run. Scenarios each have a directory of
/// their own, so it's the scenario's run even on the shared server.
pub async fn run_started_in(address: &str, directory: &Path) -> String {
    // marche records the directory as the system reports it, with symbolic
    // links resolved.
    let directory = directory
        .canonicalize()
        .expect("resolve the scenario's directory");
    let query = format!(
        "MarcheDirectory = '{}' AND ExecutionStatus = 'Running'",
        directory.display()
    );
    let mut id = None;
    let what = format!("Temporal doesn't list a run where {query}");
    wait_until(PATIENCE, &what, async || {
        let runs = list(address, &query).await;
        if let [run] = runs.as_slice() {
            id = run["execution"]["workflowId"].as_str().map(str::to_string);
        }
        id.is_some()
    })
    .await;
    id.expect("the run's ID")
}

/// The runs that visibility lists for the query.
async fn list(address: &str, query: &str) -> Vec<Value> {
    let output = Command::new("temporal")
        .args(["workflow", "list", "--address", address, "-o", "json"])
        .args(["--query", query])
        .output()
        .await
        .expect("run temporal workflow list");
    match serde_json::from_slice(&output.stdout) {
        Ok(Value::Array(runs)) => runs,
        _ => Vec::new(),
    }
}

#[given(regex = r#"^"([^"]+)" has run$"#)]
async fn has_run(world: &mut MarcheWorld, path: String) {
    let (mut command, name) = marche_beside(world, &path).await;
    let output = command
        .args(["run", &name, "--json"])
        .output()
        .await
        .expect("run marche run");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "marche run didn't print JSON ({error}):\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let id = result["run"]
        .as_str()
        .expect("marche run --json has a run ID")
        .to_string();
    let query = format!("WorkflowId = '{id}' AND ExecutionStatus != 'Running'");
    wait_listed(&world.address().await, &query).await;
    world.run_id = Some(id);
}

#[given(regex = r#"^"([^"]+)" is running its (\w+) task$"#)]
async fn is_running(world: &mut MarcheWorld, path: String, task: String) {
    let (mut command, name) = marche_beside(world, &path).await;
    let output = command
        .args(["run", &name, "--detach"])
        .output()
        .await
        .expect("run marche run --detach");
    assert!(
        output.status.success(),
        "marche run --detach failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let address = world.address().await;
    let what = format!("{task} didn't start");
    wait_until(PATIENCE, &what, async || {
        let show = Command::new(marche())
            .args(["show", &id, "--json", "--address", &address])
            .output()
            .await
            .expect("run marche show");
        let show: Value = serde_json::from_slice(&show.stdout).unwrap_or(Value::Null);
        show["tasks"].as_array().is_some_and(|tasks| {
            tasks
                .iter()
                .any(|record| record["name"] == task.as_str() && record["status"] == "running")
        })
    })
    .await;
    wait_listed(&address, &format!("WorkflowId = '{id}'")).await;
    world.run_id = Some(id);
}

#[given(regex = r#"^the run "([^"]+)" has started on a task queue no worker serves$"#)]
async fn started_without_worker(world: &mut MarcheWorld, id: String) {
    let address = world.address().await;
    let output = Command::new("temporal")
        .args(["workflow", "start", "--address", &address])
        .args(["--type", "marche.interpret", "--task-queue", "elsewhere"])
        .args(["--workflow-id", &id])
        .output()
        .await
        .expect("run temporal workflow start");
    assert!(
        output.status.success(),
        "temporal workflow start failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    wait_listed(&address, &format!("WorkflowId = '{id}'")).await;
}
