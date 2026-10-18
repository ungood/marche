//! The Temporal dev server and marche worker that scenarios run against. The
//! shared pair starts on first use, once per test run, on a free port, and
//! stops when the run ends. A scenario that kills its worker starts a pair of
//! its own, so it can't disturb the others.

use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::world::marche;

static SHARED: Mutex<Option<Services>> = Mutex::new(None);

/// What `marche worker` prints once it's ready.
const READY: &str = "marche worker is running";

#[derive(Debug)]
pub struct Services {
    pub address: String,
    pub state: PathBuf,
    server: Child,
    worker: Child,
}

/// The address of the shared dev server, starting it and the worker if needed.
pub fn address() -> String {
    let mut services = SHARED.lock().expect("services lock");
    services.get_or_insert_with(Services::start).address.clone()
}

/// The state directory the shared worker writes run logs to.
pub fn state_dir() -> PathBuf {
    let mut services = SHARED.lock().expect("services lock");
    services.get_or_insert_with(Services::start).state.clone()
}

pub fn stop() {
    drop(SHARED.lock().expect("services lock").take());
}

impl Services {
    pub fn start() -> Self {
        let port = free_port();
        let address = format!("127.0.0.1:{port}");
        let server = Command::new("temporal")
            .args(["server", "start-dev", "--headless", "--log-level", "error"])
            .args(["--ip", "127.0.0.1", "--port", &port.to_string()])
            .args(["--ui-port", &free_port().to_string()])
            .stdout(Stdio::null())
            .spawn()
            .expect("start the Temporal dev server: is the temporal CLI installed by mise?");
        wait_for(&address);
        let state = std::env::temp_dir().join(format!("marche-test-{port}"));
        std::fs::create_dir_all(&state).expect("create worker state dir");
        let worker = start_worker(&address, &state);
        Self {
            address,
            state,
            server,
            worker,
        }
    }

    /// Kills the worker without warning, as a crash would, and starts another.
    pub fn restart_worker(&mut self) {
        let _ = self.worker.kill();
        let _ = self.worker.wait();
        self.worker = start_worker(&self.address, &self.state);
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        for child in [&mut self.worker, &mut self.server] {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.state);
    }
}

/// Starts `marche worker` and waits until it's ready, passing its output on.
fn start_worker(address: &str, state: &PathBuf) -> Child {
    let mut worker = Command::new(marche())
        .args(["worker", "--address", address])
        .env("XDG_STATE_HOME", state)
        .stderr(Stdio::piped())
        .spawn()
        .expect("start marche worker");
    let stderr = worker.stderr.take().expect("worker stderr");
    let (ready, started) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.contains(READY) {
                let _ = ready.send(());
            }
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
