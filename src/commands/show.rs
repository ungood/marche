use std::process::ExitCode;

use serde_json::json;
use temporalio_client::errors::WorkflowInteractionError;
use temporalio_client::{
    Client, WorkflowDescribeOptions, WorkflowExecutionStatus, WorkflowHandle, WorkflowQueryOptions,
};

use crate::error::Error;
use crate::ows::TaskRecord;
use crate::temporal::Target;
use crate::workflow::Interpret;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// The run's workflow ID, such as examples.hello-01JAB3K4Z7Y8X9W0V1T2S3R4Q5
    run: String,
    /// Print {"run", "status", "tasks"} as JSON
    #[arg(long)]
    json: bool,
}

pub async fn show(args: Args, target: &Target) -> Result<ExitCode, Error> {
    let temporal = target.connect().await?;
    let handle = temporal
        .client
        .get_workflow_handle::<Interpret>(args.run.clone());
    let status = status(&handle, &args.run).await?;
    let tasks = tasks(&handle, &args.run).await?;
    if args.json {
        println!(
            "{}",
            json!({"run": args.run, "status": status, "tasks": tasks})
        );
    } else {
        println!("{} {status}", args.run);
        for task in &tasks {
            println!("{:>9} {}", task.status.as_str(), task.reference);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The run's status in OWS terms: running, completed, faulted or cancelled.
pub async fn status(
    handle: &WorkflowHandle<Client, Interpret>,
    run: &str,
) -> Result<&'static str, Error> {
    let description = handle
        .describe(WorkflowDescribeOptions::default())
        .await
        .map_err(|error| match error {
            WorkflowInteractionError::NotFound(_) => Error::NoRun(run.to_string()),
            other => Error::Query {
                run: run.to_string(),
                message: other.to_string(),
            },
        })?;
    Ok(match description.status() {
        WorkflowExecutionStatus::Completed => "completed",
        WorkflowExecutionStatus::Failed | WorkflowExecutionStatus::TimedOut => "faulted",
        WorkflowExecutionStatus::Canceled | WorkflowExecutionStatus::Terminated => "cancelled",
        _ => "running",
    })
}

/// Every task that has started, from the workflow's `tasks` query.
pub async fn tasks(
    handle: &WorkflowHandle<Client, Interpret>,
    run: &str,
) -> Result<Vec<TaskRecord>, Error> {
    handle
        .query(Interpret::tasks, (), WorkflowQueryOptions::default())
        .await
        .map_err(|error| Error::Query {
            run: run.to_string(),
            message: error.to_string(),
        })
}
