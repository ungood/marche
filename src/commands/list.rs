use std::process::ExitCode;

use serde_json::json;

use crate::commands::Verbosity;
use crate::error::Error;
use crate::runs::{self, local_time, tone};
use crate::temporal::Target;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// List the runs started from every directory
    #[arg(long)]
    all: bool,
    /// List only the newest runs, this many of them
    #[arg(short = 'n', long)]
    limit: Option<usize>,
    /// Print {"runs": [{"run", "definition", "status", "started", "directory"}]} as JSON
    #[arg(long)]
    json: bool,
}

pub async fn list(args: Args, target: &Target, verbosity: Verbosity) -> Result<ExitCode, Error> {
    let temporal = target.connect().await?;
    let here = if args.all {
        None
    } else {
        Some(runs::current_dir()?)
    };
    let mut runs = runs::list(&temporal, here.as_deref()).await?;
    let none = runs.is_empty();
    if let Some(limit) = args.limit {
        runs.truncate(limit);
    }

    if args.json {
        println!("{}", json!({ "runs": runs }));
        return Ok(ExitCode::SUCCESS);
    }
    if runs.is_empty() {
        if none && verbosity != Verbosity::Quiet {
            if args.all {
                eprintln!("There are no runs.");
            } else {
                eprintln!(
                    "No runs were started from this directory. `marche list --all` lists every run."
                );
            }
        }
        return Ok(ExitCode::SUCCESS);
    }
    let rows: Vec<[String; 4]> = runs
        .iter()
        .map(|run| {
            [
                run.run.clone(),
                run.definition.clone().unwrap_or_else(|| "-".into()),
                run.status.to_string(),
                run.started.map(local_time).unwrap_or_default(),
            ]
        })
        .collect();
    let width = |column: usize| rows.iter().map(|row| row[column].len()).max().unwrap_or(0);
    let (id, definition, status) = (width(0), width(1), width(2));
    for [run, def, state, started] in &rows {
        // Padded before styling, so color codes don't count toward the width.
        let state = tone(state).apply_to(format!("{state:status$}"));
        println!("{run:id$}  {def:definition$}  {state}  {started}");
    }
    Ok(ExitCode::SUCCESS)
}
