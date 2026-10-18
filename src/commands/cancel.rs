use std::process::ExitCode;
use std::time::Duration;

use serde_json::json;
use temporalio_client::errors::{WorkflowGetResultError, WorkflowInteractionError};
use temporalio_client::{WorkflowCancelOptions, WorkflowGetResultOptions};

use crate::commands::Verbosity;
use crate::error::Error;
use crate::runs;
use crate::temporal::Target;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// The run's workflow ID or ULID
    run: String,
    /// Print {"run", "status"} as JSON
    #[arg(long)]
    json: bool,
}

/// How often a cancel that's waiting checks that a worker is still serving
/// the run's task queue.
const WORKER_CHECK_INTERVAL: Duration = Duration::from_secs(5);

pub async fn cancel(args: Args, target: &Target, verbosity: Verbosity) -> Result<ExitCode, Error> {
    let temporal = target.connect().await?;
    let id = runs::resolve(&temporal, &args.run).await?;
    let run = runs::describe(&temporal, &id).await?;
    let report = |status: &str, message: String| {
        if args.json {
            println!("{}", json!({"run": id, "status": status}));
        } else {
            println!("{message}");
        }
        Ok(ExitCode::SUCCESS)
    };
    if run.status != "running" {
        return report(run.status, format!("{id} had already {}.", run.status));
    }

    let handle = runs::handle(&temporal, &id);
    handle
        .cancel(WorkflowCancelOptions::default())
        .await
        .map_err(|error| match error {
            WorkflowInteractionError::NotFound(_) => Error::NoRun(id.clone()),
            other => Error::Cancel {
                run: id.clone(),
                message: other.to_string(),
            },
        })?;
    // Only a worker can stop the run. A worker's processes die with it, so
    // without one, nothing the run started is still running.
    let no_worker = || {
        report(
            "running",
            format!(
                "Cancellation of {id} is requested. No worker is serving the task queue {}, so the run will stop once one picks it up.",
                run.task_queue
            ),
        )
    };
    if runs::worker_serves(&temporal, &run.task_queue).await == Some(false) {
        return no_worker();
    }
    if verbosity != Verbosity::Quiet && !args.json {
        eprintln!("Waiting for {id} to stop.");
    }
    // The run ends once every task in it has stopped, and shell steps stop
    // only when their processes have exited.
    let result = handle.get_result(WorkflowGetResultOptions::default());
    tokio::pin!(result);
    let mut worker_check = tokio::time::interval(WORKER_CHECK_INTERVAL);
    worker_check.tick().await;
    let result = loop {
        tokio::select! {
            result = &mut result => break result,
            _ = worker_check.tick() => {
                // The worker stopped while the run was stopping.
                if runs::worker_serves(&temporal, &run.task_queue).await == Some(false) {
                    return no_worker();
                }
            }
        }
    };
    let status = match result {
        Ok(_) => "completed",
        Err(WorkflowGetResultError::Failed(_) | WorkflowGetResultError::TimedOut) => "faulted",
        Err(
            WorkflowGetResultError::Cancelled { .. } | WorkflowGetResultError::Terminated { .. },
        ) => "cancelled",
        Err(other) => {
            return Err(Error::Wait {
                run: id.clone(),
                message: other.to_string(),
            });
        }
    };
    let message = if status == "cancelled" {
        format!("{id} was cancelled.")
    } else {
        format!("{id} {status} before it could be cancelled.")
    };
    report(status, message)
}
