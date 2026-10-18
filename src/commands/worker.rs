use std::process::ExitCode;
use std::time::Duration;

use temporalio_sdk::{Runtime, Worker, WorkerOptions};
use tokio::signal::unix::{SignalKind, signal};

use crate::commands::Verbosity;
use crate::error::Error;
use crate::shell::{HEARTBEAT_INTERVAL, ShellActivities, state_dir};
use crate::temporal::Target;
use crate::workflow::{CONTINUE_AS_NEW_AFTER, Interpret, TASK_QUEUE};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// A test hook: continue each run as new once this many tasks have
    /// finished in it, as if Temporal had suggested it
    #[arg(long, hide = true)]
    continue_as_new_after: Option<usize>,
}

pub async fn worker(args: Args, target: &Target, verbosity: Verbosity) -> Result<ExitCode, Error> {
    if let Some(after) = args.continue_as_new_after {
        let _ = CONTINUE_AS_NEW_AFTER.set(after);
    }
    let temporal = target.connect().await?;
    temporal.register_search_attributes().await?;
    let runtime = Runtime::from_current_tokio(Default::default())
        .map_err(|error| Error::WorkerStart(error.to_string()))?;
    let marche = std::env::current_exe().map_err(|error| Error::WorkerStart(error.to_string()))?;
    let options = WorkerOptions::new(TASK_QUEUE)
        .register_activities(ShellActivities {
            state_dir: state_dir(),
            marche,
            verbose: verbosity == Verbosity::Verbose,
        })
        .register_workflow::<Interpret>()
        .map_err(|error| Error::WorkerStart(error.to_string()))?
        // Running steps are killed at once on shutdown, and Temporal reruns them
        // on the next worker.
        .graceful_shutdown_period(Duration::ZERO)
        // Steps learn of cancellation from heartbeats, which the worker
        // otherwise holds back for most of the heartbeat timeout.
        .max_heartbeat_throttle_interval(HEARTBEAT_INTERVAL)
        .build();
    let mut worker = Worker::new(&runtime, temporal.client, options)
        .map_err(|error| Error::WorkerStart(error.to_string()))?;

    let shutdown = worker.shutdown_handle();
    tokio::spawn(async move {
        let mut terminate = signal(SignalKind::terminate()).expect("listen for SIGTERM");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
        shutdown();
    });
    if verbosity != Verbosity::Quiet {
        eprintln!(
            "marche worker is running on the task queue {TASK_QUEUE} in the namespace {}.",
            temporal.namespace
        );
    }
    worker
        .run()
        .await
        .map_err(|error| Error::Worker(error.to_string()))?;
    Ok(ExitCode::SUCCESS)
}
