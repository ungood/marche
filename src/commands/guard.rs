//! `marche guard`, the hidden command each shell step runs under. It starts
//! the step in a process group of its own, and holds the read end of a pipe
//! whose write end only the worker has. When the pipe closes, because the
//! worker cancelled the step or died, it stops the group: SIGTERM, then
//! SIGKILL after the grace period. Standard streams pass straight to the
//! step, and the guard exits as the step did, by the same code or signal.

use std::ffi::OsString;
use std::future::Future;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::time::Duration;

use tokio::process::{Child, Command};
use tokio::signal::unix::{SignalKind, signal};
use tokio::time::Instant;

use crate::shell::TERMINATION_GRACE;

/// How often a stopping group is checked for processes that are left.
const POLL: Duration = Duration::from_millis(50);

#[derive(Debug, clap::Args)]
pub struct Args {
    /// The read end of the worker's pipe, which closes when the step must stop
    #[arg(long)]
    lifeline: RawFd,
    /// The program to run, and its arguments
    #[arg(last = true, required = true)]
    command: Vec<OsString>,
}

pub async fn guard(args: Args) -> ! {
    // SAFETY: the worker opens this descriptor for the guard, and nothing else
    // in this process owns it.
    let lifeline = unsafe { OwnedFd::from_raw_fd(args.lifeline) };
    keep_descriptors_from_step();
    // A signal to the guard stops the step too, rather than orphaning it.
    let signals = [
        SignalKind::terminate(),
        SignalKind::interrupt(),
        SignalKind::hangup(),
    ]
    .map(|kind| signal(kind).expect("listen for signals"));

    let (program, arguments) = args.command.split_first().expect("clap requires a program");
    let child = match Command::new(program)
        .args(arguments)
        .process_group(0)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            eprintln!("marche: couldn't start {}: {error}", program.display());
            std::process::exit(127);
        }
    };
    // Only the step holds its streams now, so they end when it does.
    release_streams();

    let stop = async move {
        let [mut terminate, mut interrupt, mut hangup] = signals;
        tokio::select! {
            () = closed(lifeline) => {}
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
            _ = hangup.recv() => {}
        }
    };
    match supervise(child, stop, TERMINATION_GRACE).await {
        Ok(status) => exit_like(status),
        Err(_) => std::process::exit(127),
    }
}

/// Waits for the step, and stops its group if `stop` finishes first.
async fn supervise(
    mut child: Child,
    stop: impl Future<Output = ()>,
    grace: Duration,
) -> std::io::Result<ExitStatus> {
    let Some(group) = child.id().map(|pid| pid as libc::pid_t) else {
        return child.wait().await;
    };
    tokio::select! {
        status = child.wait() => status,
        () = stop => terminate(group, &mut child, grace).await,
    }
}

/// Sends SIGTERM to the group, then SIGKILL if any of it is still running
/// after the grace period.
async fn terminate(
    group: libc::pid_t,
    child: &mut Child,
    grace: Duration,
) -> std::io::Result<ExitStatus> {
    let deadline = Instant::now() + grace;
    // SAFETY: killpg only sends a signal, to the group this guard started.
    unsafe { libc::killpg(group, libc::SIGTERM) };
    let status = match tokio::time::timeout_at(deadline, child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            // SAFETY: as above.
            unsafe { libc::killpg(group, libc::SIGKILL) };
            return child.wait().await;
        }
    };
    // The step has exited, but processes it started may still be stopping.
    while running(group) {
        if Instant::now() >= deadline {
            // SAFETY: as above.
            unsafe { libc::killpg(group, libc::SIGKILL) };
            break;
        }
        tokio::time::sleep(POLL).await;
    }
    Ok(status)
}

/// Whether any process is left in the group.
fn running(group: libc::pid_t) -> bool {
    // SAFETY: signal 0 only checks that the group exists.
    let found = unsafe { libc::killpg(group, 0) } == 0;
    found || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// Finishes when the worker's end of the pipe closes. The worker never
/// writes to it, so a read only returns when it closes.
async fn closed(lifeline: OwnedFd) {
    let _ = tokio::task::spawn_blocking(move || {
        let mut pipe = std::fs::File::from(lifeline);
        let mut byte = [0; 1];
        loop {
            match pipe.read(&mut byte) {
                Ok(0) => break,
                Err(error) if error.kind() != std::io::ErrorKind::Interrupted => break,
                _ => {}
            }
        }
    })
    .await;
}

/// Marks every descriptor but the standard streams close-on-exec, so the step
/// doesn't inherit the lifeline, or another step's if one leaked in.
fn keep_descriptors_from_step() {
    let Ok(entries) = std::fs::read_dir("/dev/fd") else {
        return;
    };
    let descriptors: Vec<RawFd> = entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .filter(|&fd| fd > 2)
        .collect();
    for fd in descriptors {
        // SAFETY: setting a flag on a descriptor that may already be closed is
        // harmless.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
}

/// Points the guard's own standard streams at /dev/null.
fn release_streams() {
    let Ok(null) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
    else {
        return;
    };
    for fd in 0..=2 {
        // SAFETY: dup2 replaces a standard stream with /dev/null.
        unsafe { libc::dup2(null.as_raw_fd(), fd) };
    }
}

/// Exits as the step did: with its code, or killed by the same signal.
fn exit_like(status: ExitStatus) -> ! {
    if let Some(signal) = status.signal() {
        // SAFETY: these calls restore the signal's default action, without a
        // core dump, and deliver it to this process.
        unsafe {
            let no_core = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            libc::setrlimit(libc::RLIMIT_CORE, &no_core);
            libc::signal(signal, libc::SIG_DFL);
            let mut set = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, signal);
            libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
            libc::raise(signal);
        }
        // A signal whose default action isn't to exit, as shells report it.
        std::process::exit(128 + signal);
    }
    std::process::exit(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(script: &str) -> Child {
        Command::new("/bin/sh")
            .args(["-c", script])
            .process_group(0)
            .spawn()
            .expect("start sh")
    }

    #[tokio::test]
    async fn a_step_that_finishes_keeps_its_exit_code() {
        let status = supervise(start("exit 3"), std::future::pending(), TERMINATION_GRACE)
            .await
            .unwrap();
        assert_eq!(status.code(), Some(3));
    }

    #[tokio::test]
    async fn stopping_terminates_the_whole_group() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "sleep 60 & echo ready; wait $!"])
            .stdout(std::process::Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let group = child.id().unwrap() as libc::pid_t;
        // Wait until sleep has started, so there's more than sh to stop.
        let mut ready = [0; 6];
        tokio::io::AsyncReadExt::read_exact(child.stdout.as_mut().unwrap(), &mut ready)
            .await
            .unwrap();
        let status = supervise(child, async {}, TERMINATION_GRACE).await.unwrap();
        // killpg signals the group's processes one at a time, so sh can see
        // sleep die and exit as it did before its own signal lands.
        let terminated =
            status.signal() == Some(libc::SIGTERM) || status.code() == Some(128 + libc::SIGTERM);
        assert!(terminated, "{status:?}");
        assert!(!running(group));
    }

    #[tokio::test]
    async fn a_group_that_ignores_sigterm_is_killed_after_the_grace_period() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "trap '' TERM; sleep 60 & echo ready; wait $!"])
            .stdout(std::process::Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let group = child.id().unwrap() as libc::pid_t;
        // Wait until the trap is set.
        let mut ready = [0; 6];
        tokio::io::AsyncReadExt::read_exact(child.stdout.as_mut().unwrap(), &mut ready)
            .await
            .unwrap();
        let started = Instant::now();
        let status = supervise(child, async {}, Duration::from_millis(500))
            .await
            .unwrap();
        // As above, sh can see sleep die and exit before its own SIGKILL lands.
        let killed =
            status.signal() == Some(libc::SIGKILL) || status.code() == Some(128 + libc::SIGKILL);
        assert!(killed, "{status:?}");
        assert!(started.elapsed() >= Duration::from_millis(500));
        // After SIGKILL, the guard doesn't wait for the rest of the group to go.
        let deadline = Instant::now() + Duration::from_secs(1);
        while running(group) && Instant::now() < deadline {
            tokio::time::sleep(POLL).await;
        }
        assert!(!running(group));
    }
}
