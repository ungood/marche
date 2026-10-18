use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use temporalio_client::errors::{WorkflowGetResultError, WorkflowStartError};
use temporalio_client::tonic::Code;
use temporalio_client::{Client, WorkflowGetResultOptions, WorkflowHandle, WorkflowStartOptions};
use temporalio_common::error::IncomingError;
use temporalio_common::search_attributes::SearchAttributes;

use crate::commands::{Verbosity, show};
use crate::definition;
use crate::error::Error;
use crate::ows::{self, Status, TaskRecord};
use crate::runs;
use crate::temporal::{DEFINITION, DEFINITION_HASH, DIRECTORY, Target};
use crate::workflow::{Interpret, RunInput, TASK_QUEUE};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// How often a waiting run checks that a worker is serving its task queue.
const WORKER_CHECK_INTERVAL: Duration = Duration::from_secs(5);

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

pub async fn run(args: Args, target: &Target, verbosity: Verbosity) -> Result<ExitCode, Error> {
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
        resume: None,
        records: 0,
    };
    let handle = temporal
        .client
        .start_workflow(Interpret::run, run_input, options)
        .await
        .map_err(|error| start_error(error, &temporal.namespace))?;

    if args.detach {
        if args.json {
            println!("{}", json!({"run": id, "status": "running"}));
        } else {
            println!("{id}");
        }
        return Ok(ExitCode::SUCCESS);
    }

    let tasks = temporal.client.get_workflow_handle::<Interpret>(id.clone());
    let mut progress = Progress {
        verbose: verbosity == Verbosity::Verbose,
        printed: Vec::new(),
        chain: show::Chain::default(),
    };
    let quiet = verbosity == Verbosity::Quiet;
    let result = {
        let result = handle.get_result(WorkflowGetResultOptions::default());
        tokio::pin!(result);
        let mut worker_check = tokio::time::interval(WORKER_CHECK_INTERVAL);
        // Whether the last check found a worker, so the hint is printed once
        // each time the worker goes missing.
        let mut served = true;
        loop {
            tokio::select! {
                result = &mut result => break result,
                _ = worker_check.tick(), if !quiet => {
                    let now = runs::worker_serves(&temporal, TASK_QUEUE).await.unwrap_or(true);
                    if served && !now {
                        eprintln!(
                            "No worker is serving the task queue {TASK_QUEUE}, so the run is waiting for one. Start one with `marche worker`."
                        );
                    }
                    served = now;
                }
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
        // A terminated run is reported as cancelled, with its own message.
        Err(WorkflowGetResultError::Terminated { .. }) if !args.json => {
            return Err(Error::Terminated);
        }
        Err(WorkflowGetResultError::Terminated { .. }) => Err(None),
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

fn start_error(error: WorkflowStartError, namespace: &str) -> Error {
    if let WorkflowStartError::Rpc(status) = &error
        && status.code() == Code::NotFound
        && status.message().contains("amespace")
    {
        return Error::NoNamespace(namespace.to_string());
    }
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
struct Progress {
    /// Whether to print each finished task's output or error.
    verbose: bool,
    /// The status last printed for each task, by its position in the run.
    printed: Vec<Status>,
    /// The records of the runs before the latest, once it has continued as new.
    chain: show::Chain,
}

impl Progress {
    async fn print(&mut self, handle: &WorkflowHandle<Client, Interpret>, run: &str) {
        let Ok(tasks) = self.chain.tasks(handle, run).await else {
            return;
        };
        for line in self.lines(&tasks) {
            eprintln!("{line}");
        }
    }

    /// The lines for what has changed since the last call. Records are in the
    /// order tasks started, and a task finishes only after the tasks inside
    /// it, so a finished task's line waits until the lines of the tasks
    /// inside it that finished at the same time.
    fn lines(&mut self, tasks: &[TaskRecord]) -> Vec<String> {
        let mut lines = Vec::new();
        // Finished tasks whose lines wait for the tasks inside them, innermost last.
        let mut waiting: Vec<usize> = Vec::new();
        for (index, task) in tasks.iter().enumerate() {
            while let Some(&last) = waiting.last()
                && !inside(&task.reference, &tasks[last].reference)
            {
                waiting.pop();
                lines.push(self.finished(&tasks[last]));
            }
            if index == self.printed.len() {
                lines.push(format!("started {}", task.reference));
                self.printed.push(Status::Running);
            }
            if task.status != self.printed[index] {
                self.printed[index] = task.status;
                waiting.push(index);
            }
        }
        while let Some(last) = waiting.pop() {
            lines.push(self.finished(&tasks[last]));
        }
        lines
    }

    /// The line for a task that has finished: its status and reference, and
    /// with `-v`, its output or error.
    fn finished(&self, task: &TaskRecord) -> String {
        let line = format!("{} {}", task.status.as_str(), task.reference);
        if !self.verbose {
            return line;
        }
        match (&task.output, &task.error) {
            (_, Some(error)) => format!("{line}: {}", error.summary()),
            (Some(output), None) => format!("{line}: {output}"),
            (None, None) => line,
        }
    }
}

/// Whether the task at `reference` is inside the task at `outer`.
fn inside(reference: &str, outer: &str) -> bool {
    reference
        .strip_prefix(outer)
        .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(reference: &str, status: Status) -> TaskRecord {
        TaskRecord {
            reference: reference.into(),
            name: reference.rsplit('/').next().unwrap_or_default().into(),
            status,
            output: None,
            error: None,
            log: None,
        }
    }

    fn progress() -> Progress {
        Progress {
            verbose: false,
            printed: Vec::new(),
            chain: show::Chain::default(),
        }
    }

    #[test]
    fn a_task_finishes_after_the_tasks_inside_it() {
        let mut progress = progress();
        let running = [
            record("/do/0/cook", Status::Running),
            record("/do/0/cook/fork/branches/0/burn", Status::Running),
            record("/do/0/cook/fork/branches/1/simmer", Status::Running),
            record(
                "/do/0/cook/fork/branches/1/simmer/do/0/stir",
                Status::Running,
            ),
        ];
        progress.lines(&running);
        let finished = [
            record("/do/0/cook", Status::Faulted),
            record("/do/0/cook/fork/branches/0/burn", Status::Faulted),
            record("/do/0/cook/fork/branches/1/simmer", Status::Cancelled),
            record(
                "/do/0/cook/fork/branches/1/simmer/do/0/stir",
                Status::Cancelled,
            ),
            record("/do/1/serve", Status::Completed),
        ];
        assert_eq!(
            progress.lines(&finished),
            [
                "faulted /do/0/cook/fork/branches/0/burn",
                "cancelled /do/0/cook/fork/branches/1/simmer/do/0/stir",
                "cancelled /do/0/cook/fork/branches/1/simmer",
                "faulted /do/0/cook",
                "started /do/1/serve",
                "completed /do/1/serve",
            ]
        );
    }

    #[test]
    fn tasks_that_start_and_finish_together_keep_their_order() {
        let mut progress = progress();
        let tasks = [
            record("/do/0/greet", Status::Completed),
            record("/do/1/say", Status::Completed),
        ];
        assert_eq!(
            progress.lines(&tasks),
            [
                "started /do/0/greet",
                "completed /do/0/greet",
                "started /do/1/say",
                "completed /do/1/say",
            ]
        );
        assert!(progress.lines(&tasks).is_empty());
    }

    #[test]
    fn a_task_with_a_similar_name_isnt_inside() {
        assert!(inside("/do/0/cook/do/0/stir", "/do/0/cook"));
        assert!(!inside("/do/0/cooker", "/do/0/cook"));
        assert!(!inside("/do/0/cook", "/do/0/cook"));
    }
}
