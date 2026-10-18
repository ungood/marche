use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use temporalio_client::errors::{WorkflowGetResultError, WorkflowStartError};
use temporalio_client::{Client, WorkflowGetResultOptions, WorkflowHandle, WorkflowStartOptions};
use temporalio_common::error::IncomingError;
use temporalio_common::search_attributes::SearchAttributes;

use crate::commands::show;
use crate::definition;
use crate::error::Error;
use crate::ows::{self, Status};
use crate::temporal::{DEFINITION, DEFINITION_HASH, DIRECTORY, Target};
use crate::workflow::{Interpret, RunInput, TASK_QUEUE};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, clap::Args)]
pub struct Args {
    /// The workflow definition, in YAML or JSON
    file: PathBuf,
    /// The run's input, as JSON
    #[arg(long)]
    input: Option<String>,
    /// Print the run ID and return without waiting
    #[arg(short, long)]
    detach: bool,
    /// Print {"run", "status", "output"} or {"run", "status", "error"} as JSON
    #[arg(long)]
    json: bool,
}

pub async fn run(args: Args, target: &Target, quiet: bool) -> Result<ExitCode, Error> {
    let definition = definition::load(&args.file)?;
    let input = match &args.input {
        Some(input) => serde_json::from_str(input).map_err(Error::Input)?,
        None => json!({}),
    };
    let directory = std::env::current_dir()
        .map_err(|error| Error::Read {
            path: ".".into(),
            error,
        })?
        .to_string_lossy()
        .into_owned();

    let temporal = target.connect().await?;
    let id = format!(
        "{}.{}-{}",
        definition.namespace,
        definition.name,
        ulid::Ulid::generate()
    );
    let options = WorkflowStartOptions::new(TASK_QUEUE, id.clone())
        .search_attributes(SearchAttributes::new([
            DEFINITION.value_set(definition.id()),
            DEFINITION_HASH.value_set(definition.hash.clone()),
            DIRECTORY.value_set(directory.clone()),
        ]))
        .build();
    let run_input = RunInput {
        definition: definition.json,
        hash: definition.hash,
        input,
        directory,
    };
    let handle = temporal
        .client
        .start_workflow(Interpret::run, run_input, options)
        .await
        .map_err(start_error)?;

    if args.detach {
        println!("{id}");
        return Ok(ExitCode::SUCCESS);
    }

    let tasks = temporal.client.get_workflow_handle::<Interpret>(id.clone());
    let mut progress = Progress::default();
    let result = {
        let result = handle.get_result(WorkflowGetResultOptions::default());
        tokio::pin!(result);
        loop {
            tokio::select! {
                result = &mut result => break result,
                () = tokio::time::sleep(PROGRESS_INTERVAL) => {
                    if !quiet {
                        progress.print(&tasks, &id).await;
                    }
                }
            }
        }
    };
    if !quiet {
        progress.print(&tasks, &id).await;
    }

    let outcome = match result {
        Ok(output) => Ok(output),
        Err(WorkflowGetResultError::Failed(failure)) => Err(Some(fault(*failure))),
        Err(WorkflowGetResultError::Cancelled { .. }) => Err(None),
        Err(WorkflowGetResultError::Terminated { .. }) => return Err(Error::Terminated),
        Err(other) => {
            return Err(Error::Wait {
                run: id,
                message: other.to_string(),
            });
        }
    };
    if args.json {
        let report = match &outcome {
            Ok(output) => json!({"run": id, "status": "completed", "output": output}),
            Err(Some(error)) => json!({"run": id, "status": "faulted", "error": error}),
            Err(None) => json!({"run": id, "status": "cancelled"}),
        };
        println!("{report}");
        return Ok(if outcome.is_ok() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    match outcome {
        Ok(Value::String(text)) => println!("{text}"),
        Ok(output) => println!(
            "{}",
            serde_json::to_string_pretty(&output).expect("JSON serializes")
        ),
        Err(Some(error)) => {
            return Err(Error::Faulted {
                instance: error.instance.unwrap_or_else(|| "/".into()),
                title: error.title.unwrap_or(error.kind),
                detail: error.detail,
            });
        }
        Err(None) => return Err(Error::Cancelled),
    }
    Ok(ExitCode::SUCCESS)
}

fn start_error(error: WorkflowStartError) -> Error {
    let message = error.to_string();
    match [
        "MarcheDefinition",
        "MarcheDefinitionHash",
        "MarcheDirectory",
    ]
    .into_iter()
    .find(|name| message.contains(name))
    {
        Some(name) => Error::SearchAttributes(name.to_string()),
        None => Error::Start(message),
    }
}

/// The OWS error a faulted run carries in its failure's details.
fn fault(failure: IncomingError) -> ows::Error {
    if let IncomingError::Application(application) = &failure
        && let Ok(Some(error)) = application.details::<ows::Error>()
    {
        return error;
    }
    ows::Error::runtime("Run failed", failure.to_string())
}

/// Prints each task as it starts and finishes, from the run's `tasks` query.
#[derive(Default)]
struct Progress {
    /// The status last printed for each task, by its position in the run.
    printed: Vec<Status>,
}

impl Progress {
    async fn print(&mut self, handle: &WorkflowHandle<Client, Interpret>, run: &str) {
        let Ok(tasks) = show::tasks(handle, run).await else {
            return;
        };
        for (index, task) in tasks.iter().enumerate() {
            if index == self.printed.len() {
                eprintln!("started {}", task.reference);
                self.printed.push(Status::Running);
            }
            if task.status != self.printed[index] {
                eprintln!("{} {}", task.status.as_str(), task.reference);
                self.printed[index] = task.status;
            }
        }
    }
}
