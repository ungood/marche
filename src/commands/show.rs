use std::process::ExitCode;
use std::time::Duration;

use console::style;
use futures::StreamExt;
use serde_json::json;
use temporalio_client::errors::{WorkflowInteractionError, WorkflowQueryError};
use temporalio_client::tonic::{Code, Status as RpcStatus};
use temporalio_client::{
    Client, RpcOptions, WorkflowDescribeOptions, WorkflowExecutionInfo, WorkflowHandle,
    WorkflowQueryOptions,
};
use temporalio_common::protos::temporal::api::history::v1::history_event::Attributes;

use crate::commands::Verbosity;
use crate::error::Error;
use crate::ows::{Status, TaskRecord};
use crate::runs::{self, tone};
use crate::temporal::Target;
use crate::workflow::Interpret;

/// The most characters of a task's output that a line shows, without -v.
const OUTPUT_WIDTH: usize = 200;
/// How long a worker has to answer a query for a run's tasks. A worker that
/// has just stopped is still listed as polling, so its query would otherwise
/// wait for Temporal's default deadline.
const QUERY_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Debug, clap::Args)]
pub struct Args {
    /// The run's workflow ID, such as examples.hello-01JAB3K4Z7Y8X9W0V1T2S3R4Q5, or its ULID
    run: String,
    /// Print {"run", "status", "tasks"} as JSON
    #[arg(long)]
    json: bool,
}

pub async fn show(args: Args, target: &Target, verbosity: Verbosity) -> Result<ExitCode, Error> {
    let temporal = target.connect().await?;
    let id = runs::resolve(&temporal, &args.run).await?;
    let run = runs::describe(&temporal, &id).await?;
    runs::require_worker(&temporal, &run).await?;
    let tasks = tasks(&runs::handle(&temporal, &id), &id).await?;
    if args.json {
        println!(
            "{}",
            json!({"run": id, "status": run.status, "tasks": tasks})
        );
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "{} {}",
        style(&id).bold(),
        tone(run.status).apply_to(run.status)
    );
    for task in &tasks {
        let status = task.status.as_str();
        println!("  {} {}", tone(status).apply_to(status), task.reference);
        for line in details(task, verbosity) {
            println!("    {line}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// A task's output or error, and its logs, a line each. An error's detail
/// is indented under its title.
fn details(task: &TaskRecord, verbosity: Verbosity) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(output) = &task.output {
        let mut text = output.to_string();
        if verbosity != Verbosity::Verbose && text.chars().count() > OUTPUT_WIDTH {
            text = format!("{}...", text.chars().take(OUTPUT_WIDTH).collect::<String>());
        }
        lines.push(format!("output: {text}"));
    }
    if let Some(error) = &task.error {
        let title = error.title.as_deref().unwrap_or(&error.kind);
        lines.push(format!("error: {}", style(title).red()));
        if let Some(detail) = &error.detail {
            lines.extend(detail.lines().map(|line| format!("  {line}")));
        }
    }
    if let Some(log) = &task.log {
        lines.push(format!("stdout: {}", home(log)));
        if let Some(base) = log.strip_suffix(".stdout") {
            lines.push(format!("stderr: {}", home(&format!("{base}.stderr"))));
        }
    }
    lines
}

/// A path with the home directory shown as `~`.
fn home(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&format!("{home}/")) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
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
        let description = handle
            .describe(WorkflowDescribeOptions::default())
            .await
            .map_err(|error| failed(error.to_string()))?;
        let latest = description.run_id().to_string();
        let queried = |failure: QueryFailure| match failure {
            QueryFailure::NoWorker => Error::NoWorker {
                queue: description.task_queue().to_string(),
                run: run.to_string(),
            },
            QueryFailure::Other(message) => failed(message),
        };
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
            let records = query(handle, &id).await.map_err(queried)?;
            self.earlier.push((id, records));
        }
        let mut tasks = Vec::new();
        for (_, records) in &self.earlier {
            merge(&mut tasks, records.clone());
        }
        merge(&mut tasks, query(handle, &latest).await.map_err(queried)?);
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

/// Why a query for a run's tasks failed.
enum QueryFailure {
    /// No worker answered in time.
    NoWorker,
    Other(String),
}

async fn query(
    handle: &WorkflowHandle<Client, Interpret>,
    run_id: &str,
) -> Result<Vec<TaskRecord>, QueryFailure> {
    let options = WorkflowQueryOptions::builder()
        .rpc_options(RpcOptions::builder().timeout(QUERY_DEADLINE).build())
        .build();
    let handle = run_handle(handle, run_id);
    let query = handle.query(Interpret::tasks, (), options);
    // The deadline applies to each attempt, so the whole query has one too.
    match tokio::time::timeout(QUERY_DEADLINE * 2, query).await {
        Ok(Ok(records)) => Ok(records),
        Ok(Err(WorkflowQueryError::Rpc(status))) if no_worker(&status) => {
            Err(QueryFailure::NoWorker)
        }
        Ok(Err(error)) => Err(QueryFailure::Other(error.to_string())),
        Err(_) => Err(QueryFailure::NoWorker),
    }
}

/// Whether a query failed because no worker took it: it ran out of time, which
/// the client or the server may report as the call being cancelled, or the
/// server knows of no worker polling for it.
fn no_worker(status: &RpcStatus) -> bool {
    match status.code() {
        Code::DeadlineExceeded | Code::Cancelled => true,
        Code::FailedPrecondition => status.message().contains("no poller"),
        _ => false,
    }
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
