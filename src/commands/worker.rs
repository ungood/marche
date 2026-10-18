use std::process::ExitCode;
use std::time::Duration;

use temporalio_sdk::{Runtime, Worker, WorkerOptions};
use tokio::signal::unix::{SignalKind, signal};

use crate::error::Error;
use crate::shell::{ShellActivities, state_dir};
use crate::temporal::Target;
use crate::workflow::{Interpret, TASK_QUEUE};

pub async fn worker(target: &Target) -> Result<ExitCode, Error> {
    let temporal = target.connect().await?;
    temporal.register_search_attributes().await?;
    let runtime = Runtime::from_current_tokio(Default::default())
        .map_err(|error| Error::Worker(error.to_string()))?;
    let options = WorkerOptions::new(TASK_QUEUE)
        .register_activities(ShellActivities {
            state_dir: state_dir(),
        })
        .register_workflow::<Interpret>()
        .map_err(|error| Error::Worker(error.to_string()))?
        // Running steps are killed at once on shutdown, and Temporal reruns them
        // on the next worker.
        .graceful_shutdown_period(Duration::ZERO)
        .build();
    let mut worker = Worker::new(&runtime, temporal.client, options)
        .map_err(|error| Error::Worker(error.to_string()))?;

    let shutdown = worker.shutdown_handle();
    tokio::spawn(async move {
        let mut terminate = signal(SignalKind::terminate()).expect("listen for SIGTERM");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
        shutdown();
    });
    eprintln!(
        "marche worker is running on the task queue {TASK_QUEUE} in the namespace {}.",
        temporal.namespace
    );
    worker
        .run()
        .await
        .map_err(|error| Error::Worker(error.to_string()))?;
    Ok(ExitCode::SUCCESS)
}
