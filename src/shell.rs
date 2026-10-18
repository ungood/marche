//! The `marche.shell` activity, which runs a `run: shell` step.

use std::collections::BTreeMap;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use temporalio_macros::activities;
use temporalio_sdk::ApplicationFailure;
use temporalio_sdk::activities::{ActivityContext, ActivityError};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::ows;

/// The most of each stream a step's result keeps. The log keeps all of it.
const CAPTURE_LIMIT: usize = 256 * 1024;
/// How much of standard error a failed step's error detail quotes.
const DETAIL_LIMIT: usize = 2 * 1024;
/// How often a running step heartbeats. The worker sends heartbeats at most
/// this far apart, and a step learns it was cancelled from the reply.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
/// How long a cancelled step has between SIGTERM and SIGKILL.
pub const TERMINATION_GRACE: Duration = Duration::from_secs(10);
/// How long a cancelled step's guard has to stop it, beyond the grace period,
/// before the activity stops waiting.
const GUARD_MARGIN: Duration = Duration::from_secs(5);

/// A shell step with its expressions evaluated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShellRequest {
    pub reference: String,
    /// The ID of the step's task record, unique within the run's chain, which
    /// gives each run of the step logs of its own.
    pub record: usize,
    pub command: String,
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub stdin: Option<String>,
    pub directory: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShellResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    /// The path of the standard output log. Standard error is beside it.
    pub log: Option<String>,
}

impl ShellResult {
    /// The detail of the error a non-zero exit code raises.
    pub fn failure(&self) -> String {
        let mut detail = format!("The command exited with code {}.", self.code);
        let stderr = self.stderr.trim_end();
        if !stderr.is_empty() {
            let mut start = stderr.len().saturating_sub(DETAIL_LIMIT);
            while !stderr.is_char_boundary(start) {
                start += 1;
            }
            detail.push_str(" Standard error ended with:\n");
            detail.push_str(&stderr[start..]);
        }
        detail
    }
}

/// Where marche keeps run logs: `$XDG_STATE_HOME/marche`, or
/// `~/.local/state/marche`.
pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from(".local/state"))
        .join("marche")
}

pub struct ShellActivities {
    pub state_dir: PathBuf,
    /// The marche binary, which runs each step under `marche guard`.
    pub marche: PathBuf,
    /// Whether to print each step as it starts and finishes, for `marche worker -v`.
    pub verbose: bool,
}

#[activities]
impl ShellActivities {
    #[activity(name = "marche.shell")]
    pub async fn shell(
        self: Arc<Self>,
        ctx: ActivityContext,
        request: ShellRequest,
    ) -> Result<ShellResult, ActivityError> {
        let info = ctx.info();
        let run = info.workflow_id.clone().unwrap_or_default();
        let logs = self
            .state_dir
            .join("runs")
            .join(&run)
            .join(request.reference.trim_start_matches('/'))
            .join(request.record.to_string());
        let attempt = info.attempt;
        let reference = &request.reference;
        if self.verbose {
            let attempt = match attempt {
                0 | 1 => String::new(),
                n => format!(" (attempt {n})"),
            };
            eprintln!("{run} started {reference}{attempt}: {}", request.command);
        }
        let result = run_step(&ctx, &self.marche, &request, &logs, attempt).await;
        if self.verbose {
            match &result {
                Ok(result) => {
                    eprintln!("{run} finished {reference} with exit code {}", result.code)
                }
                Err(ActivityError::Cancelled { .. }) => eprintln!("{run} cancelled {reference}"),
                Err(_) => eprintln!("{run} couldn't start {reference}"),
            }
        }
        result
    }
}

fn failed_to_start(request: &ShellRequest, error: std::io::Error) -> ActivityError {
    let error = ows::Error::runtime(
        "Command failed to start",
        format!(
            "Couldn't run the command in {}: {error}.",
            request.directory
        ),
    );
    ActivityError::Application(Box::new(
        ApplicationFailure::builder(error.clone())
            .type_name(error.kind.clone())
            .non_retryable(true)
            .details(error)
            .build(),
    ))
}

async fn run_step(
    ctx: &ActivityContext,
    marche: &Path,
    request: &ShellRequest,
    logs: &Path,
    attempt: u32,
) -> Result<ShellResult, ActivityError> {
    let start = |error| failed_to_start(request, error);
    tokio::fs::create_dir_all(logs).await.map_err(start)?;
    let stdout_log = logs.join(format!("{attempt}.stdout"));
    let stderr_log = logs.join(format!("{attempt}.stderr"));

    // The guard stops the step when this pipe's write end closes: when the
    // step is cancelled, or when the worker dies, however it dies.
    let (lifeline, lifeline_held) = std::io::pipe().map_err(start)?;
    let fd = lifeline.as_raw_fd();
    let mut command = Command::new(marche);
    command
        .args([
            "guard",
            "--lifeline",
            &fd.to_string(),
            "--",
            "/bin/sh",
            "-c",
        ])
        .arg(&request.command)
        .arg("sh")
        .args(&request.arguments)
        .current_dir(&request.directory)
        .envs(&request.environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A group of its own, so signals to the worker's group pass it by.
        .process_group(0);
    // SAFETY: fcntl is async-signal-safe. It clears close-on-exec on the
    // pipe's read end, so the guard inherits it.
    unsafe {
        command.pre_exec(move || {
            if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(start)?;
    drop(lifeline);

    let stdin = child.stdin.take();
    let input = request.stdin.clone().unwrap_or_default();
    let feed = async move {
        if let Some(mut stdin) = stdin {
            // A command that doesn't read its input closes the pipe early.
            let _ = stdin.write_all(input.as_bytes()).await;
        }
    };
    let stdout = capture(child.stdout.take(), stdout_log.clone());
    let stderr = capture(child.stderr.take(), stderr_log);
    let finished = async {
        let (status, (), stdout, stderr) = tokio::join!(child.wait(), feed, stdout, stderr);
        (status, stdout, stderr)
    };
    tokio::pin!(finished);

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        tokio::select! {
            (status, stdout, stderr) = &mut finished => {
                let status = status.map_err(start)?;
                return Ok(ShellResult {
                    // A process killed by a signal has no code; shells report 128 + signal.
                    code: status.code().unwrap_or_else(|| {
                        use std::os::unix::process::ExitStatusExt;
                        128 + status.signal().unwrap_or(0)
                    }),
                    stdout: text(stdout),
                    stderr: text(stderr),
                    log: Some(stdout_log.to_string_lossy().into_owned()),
                });
            }
            () = ctx.cancelled() => {
                drop(lifeline_held);
                // The guard may take its whole grace period to stop the step.
                // Heartbeats go on meanwhile, or Temporal would time the step
                // out and report it lost rather than cancelled.
                let stopping = tokio::time::sleep(TERMINATION_GRACE + GUARD_MARGIN);
                tokio::pin!(stopping);
                loop {
                    tokio::select! {
                        _ = &mut finished => break,
                        () = &mut stopping => break,
                        _ = heartbeat.tick() => {
                            let _ = ctx.record_heartbeat(()).await;
                        }
                    }
                }
                return Err(ActivityError::cancelled());
            }
            _ = heartbeat.tick() => {
                let _ = ctx.record_heartbeat(()).await;
            }
        }
    }
}

/// Copies a stream to its log, and keeps the last `CAPTURE_LIMIT` bytes.
async fn capture(stream: Option<impl AsyncRead + Unpin>, log: PathBuf) -> Vec<u8> {
    let Some(mut stream) = stream else {
        return Vec::new();
    };
    let mut file = tokio::fs::File::create(&log).await.ok();
    let mut kept = Vec::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if let Some(file) = file.as_mut() {
                    let _ = file.write_all(&buffer[..n]).await;
                }
                kept.extend_from_slice(&buffer[..n]);
                if kept.len() > CAPTURE_LIMIT {
                    kept.drain(..kept.len() - CAPTURE_LIMIT);
                }
            }
        }
    }
    if let Some(mut file) = file {
        let _ = file.flush().await;
    }
    kept
}

/// A stream as text, with one trailing newline removed, like `$(...)`.
fn text(bytes: Vec<u8>) -> String {
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_removes_one_trailing_newline() {
        assert_eq!(text(b"a\n\n".to_vec()), "a\n");
        assert_eq!(text(b"a".to_vec()), "a");
    }

    #[test]
    fn failure_quotes_the_end_of_standard_error() {
        let result = ShellResult {
            code: 3,
            stdout: String::new(),
            stderr: "no such branch\n".into(),
            log: None,
        };
        assert_eq!(
            result.failure(),
            "The command exited with code 3. Standard error ended with:\nno such branch"
        );
    }
}
