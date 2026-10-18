use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

use cucumber::World;
use serde_json::Value;
use tempfile::TempDir;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

use super::services::Services;

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct MarcheWorld {
    root: TempDir,
    /// The result of the last `I run`.
    pub last: Option<Output>,
    /// Whether the last `I run ... for <n> seconds` was still running when stopped.
    pub still_running: bool,
    /// The workflow definition from `a workflow with definition:`.
    pub definition: Option<String>,
    /// The input from `the workflow input is:`.
    pub input: Option<Value>,
    /// `marche run --json` output from `the workflow is executed`.
    pub run: Option<Value>,
    /// `marche show --json` output for the same run.
    pub show: Option<Value>,
    /// A server and worker of the scenario's own, for scenarios that kill the worker.
    pub services: Option<Services>,
    /// A `marche run --json` still in progress.
    pub pending: Option<Pending>,
    /// Environment variables from `the environment variable ... is ...`.
    pub env: Vec<(String, String)>,
    /// The ID of the run started last by `has run`, `is running`, or `the
    /// workflow is running`.
    pub run_id: Option<String>,
    /// How long `the workflow is executed` took, or how long the run took to
    /// stop after `the workflow is cancelled`.
    pub took: Option<std::time::Duration>,
}

/// A `marche run --json` started in the background, and everything it has
/// printed on standard error, which a task reads so the pipe never fills.
#[derive(Debug)]
pub struct Pending {
    pub child: Child,
    pub stderr: JoinHandle<String>,
}

impl MarcheWorld {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("create temp dir");
        for dir in ["work", "state"] {
            std::fs::create_dir_all(root.path().join(dir)).expect("create test dir");
        }
        Self {
            root,
            last: None,
            still_running: false,
            definition: None,
            input: None,
            run: None,
            show: None,
            services: None,
            pending: None,
            env: Vec::new(),
            run_id: None,
            took: None,
        }
    }

    /// Kills a run still in progress, and stops the scenario's own server and worker.
    pub fn stop(&mut self) {
        if let Some(mut run) = self.pending.take() {
            // Tokio reaps it in the background.
            let _ = run.child.start_kill();
        }
        self.services = None;
    }

    /// The directory commands run in.
    pub fn work_dir(&self) -> PathBuf {
        self.root.path().join("work")
    }

    /// Temporal's configuration file, so tests never read the real one.
    pub fn temporal_config(&self) -> PathBuf {
        self.root.path().join("temporal.toml")
    }

    /// Where marche keeps run logs, so tests never write to the real home directory.
    pub fn state_dir(&self) -> PathBuf {
        self.root.path().join("state")
    }

    /// The address of the scenario's own server, or else the shared one.
    pub async fn address(&self) -> String {
        match &self.services {
            Some(services) => services.address.clone(),
            None => super::services::shared().await.0,
        }
    }

    /// `text` with `$RUN` and `$ULID` replaced by the ID of the scenario's run
    /// and its ULID.
    pub fn expand(&self, text: &str) -> String {
        let Some(run) = &self.run_id else {
            return text.to_string();
        };
        let ulid = run.rsplit('-').next().unwrap_or(run);
        text.replace("$RUN", run).replace("$ULID", ulid)
    }

    pub fn last(&self) -> &Output {
        self.last.as_ref().expect("no command has run")
    }

    pub fn write(&self, name: &str, content: &str) -> PathBuf {
        let path = self.work_dir().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create test directory");
        }
        std::fs::write(&path, content).expect("write test file");
        path
    }
}

/// How long a command that should end by itself may take.
pub const COMMAND_PATIENCE: Duration = Duration::from_secs(180);

/// Runs a command, and fails the step if it doesn't end within
/// `COMMAND_PATIENCE`.
pub async fn output_of(command: &mut Command) -> Output {
    let (output, stopped) = output_within(command, COMMAND_PATIENCE).await;
    assert!(
        !stopped,
        "{command:?} didn't end within {COMMAND_PATIENCE:?}:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Runs a command for up to `limit`, and stops it if it's still running then:
/// with SIGTERM, and SIGKILL five seconds later. Says whether it was stopped.
pub async fn output_within(command: &mut Command, limit: Duration) -> (Output, bool) {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap_or_else(|error| panic!("start {command:?}: {error}"));
    let mut stdout = child.stdout.take().expect("standard output");
    let mut stderr = child.stderr.take().expect("standard error");
    let stdout = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes).await;
        bytes
    });
    let stderr = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes).await;
        bytes
    });
    let (status, stopped) = match tokio::time::timeout(limit, child.wait()).await {
        Ok(status) => (status.expect("wait for the command"), false),
        Err(_) => {
            if let Some(pid) = child.id() {
                let pid = libc::pid_t::try_from(pid).expect("process ID");
                // SAFETY: kill only sends a signal, to a process this test started.
                unsafe { libc::kill(pid, libc::SIGTERM) };
            }
            let status = match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
                Ok(status) => status.expect("wait for the command"),
                Err(_) => {
                    let _ = child.start_kill();
                    child.wait().await.expect("wait for the command")
                }
            };
            (status, true)
        }
    };
    let output = Output {
        status,
        stdout: stdout.await.expect("read standard output"),
        stderr: stderr.await.expect("read standard error"),
    };
    (output, stopped)
}

/// The marche binary under test.
pub fn marche() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_marche"))
}

/// Text with colors removed and runs of whitespace collapsed, for matching messages.
/// miette's decoration, such as the bar it draws beside a wrapped message, is
/// removed too.
pub fn plain(bytes: &[u8]) -> String {
    let stripped = strip_ansi_escapes::strip(bytes);
    String::from_utf8_lossy(&stripped)
        .replace(|c| "│×╭╰─┬┴├┤╮╯·▲⚠✖‽".contains(c), " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A step's doc string. The gherkin parser keeps a media type such as `yaml`
/// on the first line, and then can't remove the indentation, so this does both.
pub fn docstring(step: &cucumber::gherkin::Step) -> String {
    let raw = step
        .docstring
        .as_deref()
        .expect("the step has a doc string");
    let body = raw.split_once('\n').map_or("", |(_, body)| body);
    let indent = body
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    body.lines()
        .map(|line| line.get(indent..).unwrap_or("").trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}
