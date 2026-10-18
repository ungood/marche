use std::process::ExitCode;

use console::style;
use serde_json::json;
use temporalio_client::{Client, WorkflowHandle, WorkflowQueryOptions};

use crate::commands::Verbosity;
use crate::error::Error;
use crate::ows::TaskRecord;
use crate::runs::{self, tone};
use crate::temporal::Target;
use crate::workflow::Interpret;

/// The most characters of a task's output that a line shows, without -v.
const OUTPUT_WIDTH: usize = 200;

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
