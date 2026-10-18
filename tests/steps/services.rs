//! The Temporal dev server and marche worker that conformance scenarios run
//! against. Both start on first use, once per test run, on a free port, and
//! stop when the run ends.

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::world::marche;

static SERVICES: Mutex<Option<Services>> = Mutex::new(None);

struct Services {
    address: String,
    state: PathBuf,
    server: Child,
    worker: Child,
}

/// The address of the running dev server, starting it and the worker if needed.
pub fn address() -> String {
    let mut services = SERVICES.lock().expect("services lock");
    services.get_or_insert_with(start).address.clone()
}

/// The state directory the worker writes run logs to.
pub fn state_dir() -> PathBuf {
    let mut services = SERVICES.lock().expect("services lock");
    services.get_or_insert_with(start).state.clone()
}

pub fn stop() {
    if let Some(mut services) = SERVICES.lock().expect("services lock").take() {
        for child in [&mut services.worker, &mut services.server] {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&services.state);
    }
}

fn start() -> Services {
    let port = free_port();
    let address = format!("127.0.0.1:{port}");
    let server = Command::new("temporal")
        .args(["server", "start-dev", "--headless", "--log-level", "error"])
        .args(["--ip", "127.0.0.1", "--port", &port.to_string()])
        .stdout(Stdio::null())
        .spawn()
        .expect("start the Temporal dev server: is the temporal CLI installed by mise?");
    wait_for(&address);
    let state = std::env::temp_dir().join(format!("marche-test-{port}"));
    std::fs::create_dir_all(&state).expect("create worker state dir");
    let worker = Command::new(marche())
        .args(["worker", "--address", &address])
        .env("XDG_STATE_HOME", &state)
        .spawn()
        .expect("start marche worker");
    Services {
        address,
        state,
        server,
        worker,
    }
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
