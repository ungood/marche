//! The Temporal workflow `marche.interpret`, which runs the interpreter and
//! records each task for `marche show`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::FutureExt;
use futures::future::LocalBoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use temporalio_common::RetryPolicy;
use temporalio_common::error::IncomingError;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{
    ActivityCancellationType, ActivityExecutionError, ActivityOptions, ApplicationFailure,
    TimerOptions, TimerResult, WorkflowContext, WorkflowContextView, WorkflowResult,
    WorkflowTermination,
};

use crate::interpreter::{Halt, Host, Interpreter, Token};
use crate::ows::{self, Status, TaskRecord};
use crate::shell::{ShellActivities, ShellRequest, ShellResult};

pub const TASK_QUEUE: &str = "marche";
/// Attempts at a shell step when its worker dies or stops heartbeating.
const SHELL_ATTEMPTS: u32 = 3;
const SHELL_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(10);
/// Shell steps have no time limit of their own; a task `timeout` sets one.
const SHELL_TIMEOUT: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// The workflow input: everything a run needs, so it never reads the file again.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunInput {
    pub definition: Value,
    /// The sha256 of the definition's JSON.
    pub hash: String,
    pub input: Value,
    /// The directory `marche run` was started in.
    pub directory: String,
}

#[workflow]
#[derive(Default)]
pub struct Interpret {
    tasks: Vec<TaskRecord>,
}

#[workflow_methods]
impl Interpret {
    #[run(name = "marche.interpret")]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: RunInput) -> WorkflowResult<Value> {
        let host = WorkflowHost { ctx: ctx.clone() };
        let id = ctx.workflow_id().to_string();
        let mut interpreter = Interpreter::new(&host, input.definition, &id, &input.directory);
        // The run's root scope: Temporal cancels it when the run is cancelled.
        let token = ctx.cancellation_token();
        match interpreter.run(input.input, &token).await {
            Ok(output) => Ok(output),
            Err(Halt::Cancelled) => Err(WorkflowTermination::cancelled()),
            Err(Halt::Fault(error)) => Err(ApplicationFailure::builder(error.clone())
                .type_name(error.kind.clone())
                .non_retryable(true)
                .details(error)
                .build()
                .into()),
        }
    }

    /// Every task that has started, in the order it started.
    #[query]
    pub fn tasks(&self, _ctx: &WorkflowContextView) -> Vec<TaskRecord> {
        self.tasks.clone()
    }
}

struct WorkflowHost {
    ctx: WorkflowContext<Interpret>,
}

impl Host for WorkflowHost {
    fn now(&self) -> SystemTime {
        self.ctx.workflow_time().unwrap_or(UNIX_EPOCH)
    }

    fn task_started(&self, reference: &str, name: &str) -> usize {
        self.ctx.state_mut(|state| {
            state.tasks.push(TaskRecord {
                reference: reference.to_string(),
                name: name.to_string(),
                status: Status::Running,
                output: None,
                error: None,
                log: None,
            });
            state.tasks.len() - 1
        })
    }

    fn task_finished(&self, record: usize, result: &Result<Value, Halt>) {
        self.ctx.state_mut(|state| {
            let Some(task) = state.tasks.get_mut(record) else {
                return;
            };
            match result {
                Ok(output) => {
                    task.status = Status::Completed;
                    task.output = Some(output.clone());
                }
                Err(Halt::Fault(error)) => {
                    task.status = Status::Faulted;
                    task.error = Some(error.clone());
                }
                Err(Halt::Cancelled) => task.status = Status::Cancelled,
            }
        });
    }

    fn timer(&self, duration: Duration, token: &Token) -> LocalBoxFuture<'_, Result<(), Halt>> {
        // Temporal rejects timers that don't wait at all.
        if duration.is_zero() {
            let result = if token.is_cancelled() {
                Err(Halt::Cancelled)
            } else {
                Ok(())
            };
            return async move { result }.boxed_local();
        }
        let options = TimerOptions::builder(duration)
            .cancellation_token(token.clone())
            .build();
        let timer = self.ctx.timer(options);
        async move {
            match timer.await {
                TimerResult::Fired => Ok(()),
                TimerResult::Cancelled => Err(Halt::Cancelled),
            }
        }
        .boxed_local()
    }

    fn random(&self, stream: &str) -> f64 {
        self.ctx.random_stream(stream).random::<f64>()
    }

    fn continue_as_new_suggested(&self) -> bool {
        self.ctx.continue_as_new_suggested()
    }

    fn shell(
        &self,
        request: ShellRequest,
        token: &Token,
    ) -> LocalBoxFuture<'_, Result<ShellResult, Halt>> {
        let reference = request.reference.clone();
        let options = ActivityOptions::with_start_to_close_timeout(SHELL_TIMEOUT)
            .heartbeat_timeout(SHELL_HEARTBEAT_TIMEOUT)
            .cancellation_type(ActivityCancellationType::WaitCancellationCompleted)
            .retry_policy(
                RetryPolicy::builder()
                    .maximum_attempts(SHELL_ATTEMPTS)
                    .build(),
            )
            .summary(reference.clone())
            .cancellation_token(token.clone())
            .build();
        async move {
            let result = self
                .ctx
                .execute_activity(ShellActivities::shell, request, options)
                .await;
            let result = match result {
                Ok(result) => Ok(result),
                Err(ActivityExecutionError::Cancelled(_)) => Err(Halt::Cancelled),
                Err(error) => Err(Halt::Fault(shell_error(&error))),
            };
            if let Ok(ShellResult { log: Some(log), .. }) = &result {
                self.ctx.state_mut(|state| {
                    if let Some(task) = state
                        .tasks
                        .iter_mut()
                        .rev()
                        .find(|t| t.reference == reference)
                    {
                        task.log = Some(log.clone());
                    }
                });
            }
            result
        }
        .boxed_local()
    }
}

/// The OWS error for a shell activity that failed: the error the activity
/// raised, or a runtime error saying the step's worker kept failing.
fn shell_error(error: &ActivityExecutionError) -> ows::Error {
    let mut cause = error.cause();
    while let Some(incoming) = cause {
        if let IncomingError::Application(failure) = incoming
            && let Ok(Some(error)) = failure.details::<ows::Error>()
        {
            return error;
        }
        cause = incoming.cause();
    }
    ows::Error::runtime(
        "Step lost",
        format!(
            "The step stopped {SHELL_ATTEMPTS} times without finishing, because its worker died or stopped responding: {error}"
        ),
    )
}
