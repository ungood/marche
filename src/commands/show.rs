use std::process::ExitCode;

use futures::StreamExt;
use serde_json::json;
use temporalio_client::errors::WorkflowInteractionError;
use temporalio_client::{
    Client, WorkflowDescribeOptions, WorkflowExecutionInfo, WorkflowExecutionStatus,
    WorkflowHandle, WorkflowQueryOptions,
};
use temporalio_common::protos::temporal::api::history::v1::history_event::Attributes;

use crate::error::Error;
use crate::ows::{Status, TaskRecord};
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

/// Every task that has started, in every Temporal run of the chain that
/// continuing as new makes.
pub async fn tasks(
    handle: &WorkflowHandle<Client, Interpret>,
    run: &str,
) -> Result<Vec<TaskRecord>, Error> {
    Chain::default().tasks(handle, run).await
}

/// The task records of a run's chain of Temporal runs. Each run answers the
/// `tasks` query with the tasks it started. Runs that continued as new never
/// change, so their records are kept between calls.
#[derive(Default)]
pub struct Chain {
    /// The runs that continued as new, oldest first, with their records.
    earlier: Vec<(String, Vec<TaskRecord>)>,
}

impl Chain {
    pub async fn tasks(
        &mut self,
        handle: &WorkflowHandle<Client, Interpret>,
        run: &str,
    ) -> Result<Vec<TaskRecord>, Error> {
        let failed = |message: String| Error::Query {
            run: run.to_string(),
            message,
        };
        let latest = handle
            .describe(WorkflowDescribeOptions::default())
            .await
            .map_err(|error| failed(error.to_string()))?
            .run_id()
            .to_string();
        // Walk back from the latest run to the runs already known. Each run's
        // first event names the run it continued from.
        let mut newer = Vec::new();
        let mut previous = continued_from(handle, &latest)
            .await
            .map_err(|error| failed(error.to_string()))?;
        while let Some(id) = previous.filter(|id| !self.earlier.iter().any(|(run, _)| run == id)) {
            previous = continued_from(handle, &id)
                .await
                .map_err(|error| failed(error.to_string()))?;
            newer.push(id);
        }
        for id in newer.into_iter().rev() {
            let records = query(handle, &id).await.map_err(failed)?;
            self.earlier.push((id, records));
        }
        let mut tasks = Vec::new();
        for (_, records) in &self.earlier {
            merge(&mut tasks, records.clone());
        }
        merge(&mut tasks, query(handle, &latest).await.map_err(failed)?);
        Ok(tasks)
    }
}

/// A handle on one Temporal run of the workflow.
fn run_handle(
    handle: &WorkflowHandle<Client, Interpret>,
    run_id: &str,
) -> WorkflowHandle<Client, Interpret> {
    let info = handle.info();
    WorkflowHandle::new(
        handle.client().clone(),
        WorkflowExecutionInfo::builder()
            .namespace(info.namespace.clone())
            .workflow_id(info.workflow_id.clone())
            .run_id(run_id.to_string())
            .build(),
    )
}

/// The run that this run continued from, if any.
async fn continued_from(
    handle: &WorkflowHandle<Client, Interpret>,
    run_id: &str,
) -> Result<Option<String>, WorkflowInteractionError> {
    let mut history = run_handle(handle, run_id).fetch_history(Default::default());
    let Some(first) = history.next().await.transpose()? else {
        return Ok(None);
    };
    Ok(match first.attributes {
        Some(Attributes::WorkflowExecutionStartedEventAttributes(started)) => {
            Some(started.continued_execution_run_id).filter(|id| !id.is_empty())
        }
        _ => None,
    })
}

async fn query(
    handle: &WorkflowHandle<Client, Interpret>,
    run_id: &str,
) -> Result<Vec<TaskRecord>, String> {
    run_handle(handle, run_id)
        .query(Interpret::tasks, (), WorkflowQueryOptions::default())
        .await
        .map_err(|error| error.to_string())
}

/// Adds a run's records to those of the runs before it. A run continues as new
/// with only the tasks around its position still running, and the next run
/// starts those again before any other, so its first records replace them.
fn merge(tasks: &mut Vec<TaskRecord>, records: Vec<TaskRecord>) {
    let running: Vec<usize> = (0..tasks.len())
        .filter(|&index| tasks[index].status == Status::Running)
        .collect();
    let mut records = records.into_iter();
    for index in running {
        if let Some(record) = records.next() {
            tasks[index] = record;
        }
    }
    tasks.extend(records);
}
