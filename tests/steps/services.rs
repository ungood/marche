//! The Temporal dev server and marche worker that scenarios run against. The
//! shared pair starts on first use, once per test run, on a free port, and
//! stops when the run ends. A scenario that kills its worker starts a pair of
//! its own, so it can't disturb the others.
//!
//! Each server and worker runs in a process group of its own, under a shell
//! that kills the group once its standard input closes. Only this process
//! holds that pipe, so they die with the test run however it ends, even when
//! it's interrupted or killed. The server's shell then removes the worker's
//! state directory too.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tempfile::TempDir;

use super::world::marche;

/// The shared server and worker. The lock is async, so scenarios that need
/// them while they start wait without holding up the scenarios beside them.
static SHARED: tokio::sync::Mutex<Option<Services>> = tokio::sync::Mutex::const_new(None);

/// What `marche worker` prints once it's ready.
const READY: &str = "marche worker is running";

/// How many times to start a server, each on a new port, before giving up.
const SERVER_ATTEMPTS: usize = 5;

/// Runs its arguments in the background, and kills its process group when
/// they exit, or when its standard input reaches end of file. The reader
/// takes standard input as descriptor 3, since a background job's standard
/// input is /dev/null. Before it kills the group, it removes
/// `$MARCHE_TEST_CLEANUP`, if that's set.
const WATCH: &str = r#"exec 3<&0
"$@" 3<&- &
child=$!
{
  read -r _ <&3
  if [ -n "$MARCHE_TEST_CLEANUP" ]; then rm -rf "$MARCHE_TEST_CLEANUP"; fi
  kill -KILL 0
} &
wait "$child"
kill -KILL 0"#;

#[derive(Debug)]
pub struct Services {
    pub address: String,
    /// Where the worker writes run logs, as its `XDG_STATE_HOME`.
    state: TempDir,
    /// Everything the worker has printed on standard error.
    pub log: Arc<Mutex<String>>,
    /// Arguments after `marche worker --address <address>`.
    args: Vec<String>,
    server: Process,
    worker: Process,
}

/// The address of the shared dev server, and the state directory its worker
/// writes run logs to, starting them if needed.
pub async fn shared() -> (String, PathBuf) {
    let mut shared = SHARED.lock().await;
    if shared.is_none() {
        // A start that panics leaves nothing behind, so the next caller tries again.
        *shared = Some(Services::start().await);
    }
    let services = shared.as_ref().expect("the shared services started");
    (services.address.clone(), services.state().to_path_buf())
}

/// Stops the shared server and worker, once every scenario has finished.
pub async fn stop() {
    drop(SHARED.lock().await.take());
}

impl Services {
    /// Starts a server and worker of the scenario's own. It waits for them on
    /// another thread, so scenarios running beside it carry on.
    pub async fn start() -> Self {
        Self::start_with(Vec::new()).await
    }

    /// Starts a server, and a worker with extra arguments, such as `-v`.
    pub async fn start_with(args: Vec<String>) -> Self {
        off_thread(move || Self::start_blocking(args)).await
    }

    /// The worker's state directory, which holds run logs under `marche/runs`.
    pub fn state(&self) -> &Path {
        self.state.path()
    }

    /// Kills the worker without warning, as a crash would, and starts another.
    pub async fn restart_worker(&mut self) {
        self.worker.stop();
        let address = self.address.clone();
        let state = self.state().to_path_buf();
        let args = self.args.clone();
        let log = Arc::clone(&self.log);
        self.worker = off_thread(move || start_worker(&address, &state, &args, &log)).await;
    }

    fn start_blocking(args: Vec<String>) -> Self {
        let state = tempfile::Builder::new()
            .prefix("marche-test-")
            .tempdir()
            .expect("create worker state dir");
        let (address, server) = start_server(state.path());
        let log = Arc::default();
        let worker = start_worker(&address, state.path(), &args, &log);
        Self {
            address,
            state,
            log,
            args,
            server,
            worker,
        }
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        // The state directory is removed after this, once both have stopped.
        self.worker.stop();
        self.server.stop();
    }
}

/// Runs blocking work on a thread of its own, and passes on its panic.
pub async fn off_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|error| std::panic::resume_unwind(error.into_panic()))
}

/// A command that runs `program` under the `WATCH` shell, in a new process
/// group. Arguments added to it go to `program`.
fn watched(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(WATCH)
        .arg("sh")
        .arg(program)
        .env_remove("MARCHE_TEST_CLEANUP")
        .stdin(Stdio::piped())
        .process_group(0);
    command
}

/// A server or worker, killed with its process group when stopped or dropped.
#[derive(Debug)]
struct Process(Option<Child>);

impl Process {
    fn spawn(command: &mut Command) -> Self {
        let child = command
            .spawn()
            .unwrap_or_else(|error| panic!("start {command:?}: {error}"));
        Self(Some(child))
    }

    /// Whether the program has exited. Its watching shell exits with it.
    fn exited(&mut self) -> bool {
        self.0
            .as_mut()
            .is_none_or(|child| !matches!(child.try_wait(), Ok(None)))
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.0.take() {
            // The watching shell leads the group, so its ID is the group's.
            let group = libc::pid_t::try_from(child.id()).expect("process ID");
            // SAFETY: killpg only sends a signal, to a group this test started.
            unsafe { libc::killpg(group, libc::SIGKILL) };
            let _ = child.wait();
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Starts a dev server on a free port, and returns its address once it's
/// serving. A port is free only until a server binds it, so another server
/// can take it first. This one then exits, or else the server on the port
/// lacks the namespace this one creates, and it starts again on another port.
fn start_server(state: &Path) -> (String, Process) {
    static STARTED: AtomicUsize = AtomicUsize::new(0);
    for _ in 0..SERVER_ATTEMPTS {
        let port = free_port();
        let address = format!("127.0.0.1:{port}");
        let marker = format!(
            "marche-test-{}-{}",
            std::process::id(),
            STARTED.fetch_add(1, Ordering::Relaxed)
        );
        let mut server = Process::spawn(
            watched("temporal")
                .args(["server", "start-dev", "--headless", "--log-level", "error"])
                .args(["--ip", "127.0.0.1", "--port", &port.to_string()])
                .args(["--ui-port", &free_port().to_string()])
                .args(["--namespace", &marker])
                .env("MARCHE_TEST_CLEANUP", state)
                .stdout(Stdio::null()),
        );
        if serving(&mut server, &address, &marker) {
            return (address, server);
        }
        eprintln!("the Temporal dev server didn't start on {address}, so trying another port");
    }
    panic!("the Temporal dev server didn't start in {SERVER_ATTEMPTS} attempts");
}

/// Waits until the server answers on `address` with the `marker` namespace,
/// which shows that the server there is this one. False if it exits first,
/// or doesn't answer within 30 seconds.
fn serving(server: &mut Process, address: &str, marker: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if server.exited() {
            return false;
        }
        if TcpStream::connect(address).is_ok() && has_namespace(address, marker) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn has_namespace(address: &str, namespace: &str) -> bool {
    Command::new("temporal")
        .args([
            "operator",
            "namespace",
            "describe",
            "--namespace",
            namespace,
        ])
        .args(["--address", address])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Starts `marche worker` and waits until it's ready, passing its output on
/// and keeping it in `log`.
fn start_worker(address: &str, state: &Path, args: &[String], log: &Arc<Mutex<String>>) -> Process {
    let mut worker = Process::spawn(
        watched(marche())
            .args(["worker", "--address", address])
            .args(args)
            .env("XDG_STATE_HOME", state)
            .stderr(Stdio::piped()),
    );
    let stderr = worker
        .0
        .as_mut()
        .and_then(|child| child.stderr.take())
        .expect("worker stderr");
    let (ready, started) = mpsc::channel();
    let log = Arc::clone(log);
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.contains(READY) {
                let _ = ready.send(());
            }
            let mut log = log.lock().unwrap_or_else(PoisonError::into_inner);
            log.push_str(&line);
            log.push('\n');
            eprintln!("{line}");
        }
    });
    started
        .recv_timeout(Duration::from_secs(30))
        .expect("marche worker didn't start");
    worker
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("find a free port");
    listener.local_addr().expect("local address").port()
}
