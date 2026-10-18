use std::process::ExitCode;

use console::style;
use serde_json::json;

use crate::commands::show;
use crate::error::Error;
use crate::ows::{Status, TaskRecord};
use crate::runs::{self, local_time, tone};
use crate::temporal::Target;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// The run's workflow ID or ULID [default: the latest run started from this directory]
    run: Option<String>,
    /// Print {"run", "definition", "status", "started", "running"} as JSON
    #[arg(long)]
    json: bool,
}

pub async fn status(args: Args, target: &Target) -> Result<ExitCode, Error> {
    let temporal = target.connect().await?;
    let id = match &args.run {
        Some(run) => runs::resolve(&temporal, run).await?,
        None => {
            let here = runs::current_dir()?;
            let latest = runs::list(&temporal, Some(&here)).await?.into_iter().next();
            latest.ok_or(Error::NoRunHere)?.run
        }
    };
    let run = runs::describe(&temporal, &id).await?;
    let running = if run.status == "running" {
        runs::require_worker(&temporal, &run).await?;
        let tasks = show::tasks(&runs::handle(&temporal, &id), &id).await?;
        innermost_running(tasks)
    } else {
        Vec::new()
    };

    if args.json {
        let mut report = serde_json::to_value(&run).expect("a run is JSON");
        report["running"] = json!(running);
        report
            .as_object_mut()
            .expect("a run is an object")
            .remove("directory");
        println!("{report}");
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "{} {}",
        style(&run.run).bold(),
        tone(run.status).apply_to(run.status)
    );
    if let Some(definition) = &run.definition {
        println!("  definition: {definition}");
    }
    if let Some(started) = run.started {
        println!("  started: {}", local_time(started));
    }
    for task in &running {
        println!("  running: {}", task.reference);
    }
    Ok(ExitCode::SUCCESS)
}

/// The running tasks that aren't running only because a task inside them is.
fn innermost_running(tasks: Vec<TaskRecord>) -> Vec<TaskRecord> {
    let running: Vec<TaskRecord> = tasks
        .into_iter()
        .filter(|task| task.status == Status::Running)
        .collect();
    running
        .iter()
        .filter(|task| {
            let inside = format!("{}/", task.reference);
            !running
                .iter()
                .any(|other| other.reference.starts_with(&inside))
        })
        .cloned()
        .collect()
}
