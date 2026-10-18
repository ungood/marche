//! The Temporal dev server and marche worker that scenarios run against. The
//! shared pair starts on first use, once per test run, on a free port, and
//! stops when the run ends. A scenario that kills its worker starts a pair of
//! its own, so it can't disturb the others.
//!
//! Each server and worker runs in a process group of its own, under a shell
//! that kills the group once its standard input closes. Only this process
//! holds that pipe, so they die with the test run however it ends, even when
//! it's interrupted or killed.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::world::marche;

static SHARED: Mutex<Option<Services>> = Mutex::new(None);

/// What `marche worker` prints once it's ready.
const READY: &str = "marche worker is running";

/// Runs its arguments in the background, then kills its process group when
/// its standard input reaches end of file.
const WATCH: &str = r#""$@" & read -r _; kill -KILL 0"#;

#[derive(Debug)]
pub struct Services {
    pub address: String,
    pub state: PathBuf,
    /// Everything the worker has printed on standard error.
    pub log: Arc<Mutex<String>>,
    /// Arguments after `marche worker --address <address>`.
    args: Vec<String>,
    server: Process,
    worker: Process,
}

/// The address of the shared dev server, starting it and the worker if needed.
pub fn address() -> String {
    shared()
        .get_or_insert_with(|| Services::start_blocking(Vec::new()))
        .address
        .clone()
}

/// The state directory the shared worker writes run logs to.
pub fn state_dir() -> PathBuf {
    shared()
        .get_or_insert_with(|| Services::start_blocking(Vec::new()))
        .state
        .clone()
}

pub fn stop() {
    drop(shared().take());
}

/// The shared services. A step that panicked while holding the lock, such as
/// when they failed to start, leaves nothing half-started behind, so the lock
/// is still good to use.
fn shared() -> MutexGuard<'static, Option<Services>> {
    SHARED.lock().unwrap_or_else(PoisonError::into_inner)
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

    /// Kills the worker without warning, as a crash would, and starts another.
    pub async fn restart_worker(&mut self) {
        self.worker.stop();
        let address = self.address.clone();
        let state = self.state.clone();
        let args = self.args.clone();
        let log = Arc::clone(&self.log);
        self.worker = off_thread(move || start_worker(&address, &state, &args, &log)).await;
    }

    fn start_blocking(args: Vec<String>) -> Self {
        let port = free_port();
        let address = format!("127.0.0.1:{port}");
        let server = Process::spawn(
            watched("temporal")
                .args(["server", "start-dev", "--headless", "--log-level", "error"])
                .args(["--ip", "127.0.0.1", "--port", &port.to_string()])
                .args(["--ui-port", &free_port().to_string()])
                .stdout(Stdio::null()),
        );
        wait_for(&address);
        let state = std::env::temp_dir().join(format!("marche-test-{port}"));
        std::fs::create_dir_all(&state).expect("create worker state dir");
        let log = Arc::default();
        let worker = start_worker(&address, &state, &args, &log);
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
        self.worker.stop();
        self.server.stop();
        let _ = std::fs::remove_dir_all(&self.state);
    }
}

/// Runs blocking work on a thread of its own, and passes on its panic.
async fn off_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
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

/// Starts `marche worker` and waits until it's ready, passing its output on
/// and keeping it in `log`.
fn start_worker(
    address: &str,
    state: &PathBuf,
    args: &[String],
    log: &Arc<Mutex<String>>,
) -> Process {
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

fn wait_for(address: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while TcpStream::connect(address).is_err() {
        assert!(
            Instant::now() < deadline,
            "the Temporal dev server didn't start on {address}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
