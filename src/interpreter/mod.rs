//! The OWS interpreter. It walks a definition's task tree and evaluates its
//! expressions, and leaves everything that touches the outside world to a
//! [`Host`]: in a run, the Temporal workflow `marche.interpret`.
//!
//! This code runs as workflow code, so it must be deterministic: no clocks,
//! randomness, environment, I/O or unordered iteration except through the host.
//! Concurrent work uses [`join_all`] and [`select`], Temporal's deterministic
//! combinators, never tokio's or futures'.
//!
//! Each task type lives in its own module under `tasks/`, and [`Interpreter::execute`]
//! dispatches to it.
//!
//! Cancellation follows the statechart rule: every scope (`do`, `fork`, `try`, a
//! `for` iteration) runs under a [`Token`], and leaving the scope cancels the
//! token, which cancels every activity and timer started under it. A child
//! token is cancelled with its parent, and the workflow's root token is
//! cancelled when the run is.

pub mod duration;
pub mod resume;
pub mod schema;
mod tasks;
#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::FutureExt;
use futures::future::LocalBoxFuture;
use serde_json::{Value, json};
pub use temporalio_sdk::WorkflowCancellationToken as Token;
pub(crate) use temporalio_sdk::workflows::join_all;
use temporalio_sdk::workflows::select;

use crate::expr::{self, Scope};
use crate::ows;
use crate::shell::{ShellRequest, ShellResult};
use resume::{Frame, State};

/// Why a task or run stopped without an output.
#[derive(Debug, Clone, PartialEq)]
pub enum Halt {
    Fault(ows::Error),
    Cancelled,
    /// The run stopped to continue as new, and
    /// [`Interpreter::continuation`] says where.
    ContinueAsNew,
}

impl From<ows::Error> for Halt {
    fn from(error: ows::Error) -> Self {
        Self::Fault(error)
    }
}

impl Halt {
    pub fn at(self, reference: &str) -> Self {
        match self {
            Self::Fault(error) => Self::Fault(error.at(reference)),
            other => other,
        }
    }
}

/// What the interpreter needs from the workflow that runs it.
pub trait Host {
    /// The workflow clock, which replays identically.
    fn now(&self) -> SystemTime;
    /// Records that a task started, and returns the record's ID, which is
    /// unique within the run, across continuing as new. A reference isn't
    /// enough to find the record again, because a loop's iterations run the
    /// same tasks, at the same time when it's parallel.
    fn task_started(&self, reference: &str, name: &str) -> usize;
    fn task_finished(&self, record: usize, result: &Result<Value, Halt>);
    /// Runs a shell step, for the task record `request.record`, which gets
    /// the step's log. Cancelling `token` stops it, and the future resolves
    /// with [`Halt::Cancelled`] once the step's processes have exited.
    fn shell(
        &self,
        request: ShellRequest,
        token: &Token,
    ) -> LocalBoxFuture<'_, Result<ShellResult, Halt>>;
    /// Waits for a duration on a durable timer. Cancelling `token` ends the wait
    /// with [`Halt::Cancelled`].
    fn timer(&self, duration: Duration, token: &Token) -> LocalBoxFuture<'_, Result<(), Halt>>;
    /// A number in [0, 1) from Temporal's seeded generator, drawn from the named
    /// stream so that one use of randomness doesn't shift another.
    fn random(&self, stream: &str) -> f64;
    /// Whether to continue as new now, as Temporal suggests when the history is
    /// long. The interpreter asks only where nothing is in flight.
    fn continue_as_new_suggested(&self) -> bool;
}

/// Where a run goes after a task, from its `then` or from a task such as
/// `switch` that picks it.
#[derive(Debug, Clone, PartialEq)]
pub enum Flow {
    Continue,
    Exit,
    End,
    Goto(String),
}

impl Flow {
    /// The flow a `then` value names.
    pub fn named(then: &str) -> Self {
        match then {
            "continue" => Self::Continue,
            "exit" => Self::Exit,
            "end" => Self::End,
            target => Self::Goto(target.to_string()),
        }
    }
}

/// The result of a task list: its output, and whether `then: end` finished the
/// whole workflow.
pub struct Done {
    pub output: Value,
    pub end: bool,
}

/// What [`Interpreter::run_task`] gives for a task that didn't halt: its
/// output, where the run goes next, and whether its `if` skipped it, in which
/// case the output is its input.
#[derive(Debug, Clone, PartialEq)]
pub struct Ran {
    pub output: Value,
    pub flow: Flow,
    pub skipped: bool,
}

/// What a task type's code gets: the task and its transformed input.
#[allow(dead_code)]
pub struct Task<'a> {
    pub name: &'a str,
    /// The task's definition, the object under its name.
    pub definition: &'a Value,
    pub reference: &'a str,
    /// The ID of the task's record, from [`Host::task_started`].
    pub record: usize,
    /// The transformed input, after `input.from`.
    pub input: Value,
    /// The expression arguments, with `$input` and `$task` set.
    pub scope: Scope,
    /// The scope this task runs in. Tasks that open a scope of their own make a
    /// `child_token()` of it.
    pub token: &'a Token,
}

/// What a task type's code returns: the raw output, before `output.as`, and the
/// flow when the task picks one itself (`None` follows the task's `then`).
pub struct Outcome {
    pub output: Value,
    pub flow: Option<Flow>,
}

impl Outcome {
    pub fn output(output: Value) -> Self {
        Self { output, flow: None }
    }
}

pub struct Interpreter<'h, H> {
    pub host: &'h H,
    definition: Value,
    workflow: Value,
    runtime: Value,
    pub directory: String,
    context: RefCell<Value>,
    /// Where a run that stops to continue as new stopped, built as it unwinds,
    /// or where a resumed run carries on, taken as it walks back down.
    frames: RefCell<VecDeque<Frame>>,
    /// How many tasks around the current point would lose something in flight
    /// by continuing as new. The run continues as new only when it's zero.
    blocked: Cell<usize>,
}

/// How long a task or workflow may run, from its `timeout.after`, which may be
/// an expression. A timeout named in `use.timeouts` has no `after`, and the
/// checks `marche run` makes reject it.
fn timeout(owner: &Value, input: &Value, scope: &Scope) -> Result<Option<Duration>, ows::Error> {
    let Some(after) = owner.pointer("/timeout/after") else {
        return Ok(None);
    };
    let after = expr::evaluate_template(after, input, scope)?;
    duration::parse(&after).map(Some)
}

/// The spec's dateTimeDescriptor.
pub fn date_time(time: SystemTime) -> Value {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let millis = since.as_millis() as i64;
    let iso8601 = jiff::Timestamp::from_millisecond(millis)
        .map(|t| t.to_string())
        .unwrap_or_default();
    json!({
        "iso8601": iso8601,
        "epoch": {"seconds": since.as_secs(), "milliseconds": millis},
    })
}

impl<'h, H: Host> Interpreter<'h, H> {
    /// `id` is the run's ID, and `directory` is where `marche run` was started.
    pub fn new(host: &'h H, definition: Value, id: &str, directory: &str) -> Self {
        Self {
            host,
            workflow: json!({"id": id, "definition": definition}),
            runtime: json!({
                "name": "marche",
                "version": env!("MARCHE_VERSION"),
                "metadata": {"directory": directory},
            }),
            definition,
            directory: directory.to_string(),
            context: RefCell::new(json!({})),
            frames: RefCell::default(),
            blocked: Cell::new(0),
        }
    }

    /// The expression arguments every expression sees.
    pub fn scope(&self) -> Scope {
        Scope {
            context: self.context.borrow().clone(),
            workflow: self.workflow.clone(),
            runtime: self.runtime.clone(),
            ..Scope::default()
        }
    }

    /// Replaces `$context`, as `export.as` does.
    pub fn set_context(&self, context: Value) {
        *self.context.borrow_mut() = context;
    }

    /// Runs the workflow on its raw input and returns its transformed output.
    /// Cancelling `token` cancels the run.
    pub async fn run(&mut self, input: Value, token: &Token) -> Result<Value, Halt> {
        self.workflow["startedAt"] = date_time(self.host.now());
        self.start(input, token).await
    }

    /// Carries on a run that continued as new, from where `state` says it
    /// stopped. `input` is the workflow's raw input, as the chain's first run
    /// got it.
    pub async fn resume(
        &mut self,
        input: Value,
        state: State,
        token: &Token,
    ) -> Result<Value, Halt> {
        self.workflow["startedAt"] = state.started_at;
        self.set_context(state.context);
        *self.frames.get_mut() = state.position.into();
        self.start(input, token).await
    }

    async fn start(&mut self, input: Value, token: &Token) -> Result<Value, Halt> {
        self.workflow["input"] = input.clone();
        let scope = self.scope();
        let definition = self.definition.clone();
        let input = if self.frames.get_mut().is_empty() {
            schema::check(&definition, "input", &input).map_err(|e| e.at("/input"))?;
            match definition.pointer("/input/from") {
                Some(from) => {
                    expr::evaluate_field(from, &input, &scope).map_err(|e| e.at("/input"))?
                }
                None => input,
            }
        } else {
            // The frames carry the current data.
            Value::Null
        };
        let after = timeout(&definition, &input, &scope).map_err(|e| e.at("/timeout"))?;
        let list = &definition["do"];
        let this = &*self;
        // A workflow timeout's timer is in flight for the whole run.
        let _blocked = after.is_some().then(|| this.block());
        let done = this
            .within(after, token, "workflow", |token| async move {
                this.run_list(list, "/do".into(), input, &token).await
            })
            .await
            .map_err(|halt| halt.at("/"))?;
        let output = match definition.pointer("/output/as") {
            Some(output) => {
                let scope = self.scope();
                expr::evaluate_field(output, &done.output, &scope).map_err(|e| e.at("/output"))?
            }
            None => done.output,
        };
        schema::check(&definition, "output", &output).map_err(|e| e.at("/output"))?;
        Ok(output)
    }

    /// Runs `work` under a child of `token`. With a duration, a timer races it:
    /// if the timer fires first, the child is cancelled, and once `work` has
    /// stopped the result is a timeout error. `what` names the task or
    /// workflow in the error.
    async fn within<T, F>(
        &self,
        after: Option<Duration>,
        token: &Token,
        what: &str,
        work: impl FnOnce(Token) -> F,
    ) -> Result<T, Halt>
    where
        F: Future<Output = Result<T, Halt>>,
    {
        let Some(after) = after else {
            return work(token.clone()).await;
        };
        let scope = token.child_token();
        let timer_scope = token.child_token();
        let mut work = pin!(work(scope.clone()).fuse());
        let mut timer = self.host.timer(after, &timer_scope).fuse();
        // Biased toward the work, so a task that finishes as the timer fires
        // completes, the same way on every replay.
        let fired = select! {
            result = work => {
                timer_scope.cancel();
                return result;
            }
            fired = timer => fired.is_ok(),
        };
        // A timer that didn't fire was cancelled with the parent scope, which
        // stops the work too.
        if !fired {
            return work.await;
        }
        scope.cancel();
        let _ = work.await;
        Err(ows::Error::timeout(format!("The {what} didn't finish within {after:?}.")).into())
    }

    /// Where a run that halted with [`Halt::ContinueAsNew`] stopped, and what
    /// the next run needs to carry on from there.
    pub fn continuation(&self) -> State {
        State {
            started_at: self.workflow["startedAt"].clone(),
            context: self.context.borrow().clone(),
            position: self.frames.borrow().iter().cloned().collect(),
        }
    }

    /// Adds a frame to the front of the position, as the run unwinds to
    /// continue as new.
    pub fn suspend(&self, frame: Frame) {
        self.frames.borrow_mut().push_front(frame);
    }

    /// Adds the frame of a `do` or `for` task that the run unwinds through.
    pub fn suspend_task(&self, task: &Task<'_>) {
        self.suspend(Frame::Task {
            input: task.scope.task["input"].clone(),
            transformed: task.input.clone(),
            started_at: task.scope.task["startedAt"].clone(),
        });
    }

    /// Takes the next frame of a resumed run, if `wanted` accepts it.
    pub fn resume_frame(&self, wanted: impl FnOnce(&Frame) -> bool) -> Option<Frame> {
        let mut frames = self.frames.borrow_mut();
        if wanted(frames.front()?) {
            frames.pop_front()
        } else {
            None
        }
    }

    /// Marks a task whose work would be lost by continuing as new, until the
    /// guard drops.
    fn block(&self) -> Blocked<'_> {
        self.blocked.set(self.blocked.get() + 1);
        Blocked(&self.blocked)
    }

    /// Whether to continue as new before the next task: when the host says so,
    /// nothing is in flight, and the run isn't still resuming.
    fn continues_as_new(&self) -> bool {
        self.blocked.get() == 0
            && self.frames.borrow().is_empty()
            && self.host.continue_as_new_suggested()
    }

    /// Runs a task list in order, following `then`. `path` is the list's
    /// reference, such as `/do` or `/do/0/outer/do`.
    pub fn run_list<'a>(
        &'a self,
        list: &'a Value,
        path: String,
        input: Value,
        token: &'a Token,
    ) -> LocalBoxFuture<'a, Result<Done, Halt>> {
        self.run_list_with(list, path, input, token, &[])
    }

    /// [`Interpreter::run_list`], with variables bound for every expression in
    /// the list's tasks, such as `catch`'s `as`.
    pub fn run_list_with<'a>(
        &'a self,
        list: &'a Value,
        path: String,
        input: Value,
        token: &'a Token,
        variables: &'a [(String, Value)],
    ) -> LocalBoxFuture<'a, Result<Done, Halt>> {
        async move {
            let items: Vec<(&String, &Value)> = list
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
                .filter_map(|item| item.iter().next())
                .collect();
            let mut index = 0;
            let mut current = input;
            let resumed = self
                .resume_frame(|frame| matches!(frame, Frame::Before { .. } | Frame::Within { .. }));
            match resumed {
                Some(Frame::Before { index: at, input }) => (index, current) = (at, input),
                Some(Frame::Within { index: at }) => index = at,
                _ => {}
            }
            while let Some((name, task)) = items.get(index) {
                if token.is_cancelled() {
                    return Err(Halt::Cancelled);
                }
                if self.continues_as_new() {
                    self.suspend(Frame::Before {
                        index,
                        input: current,
                    });
                    return Err(Halt::ContinueAsNew);
                }
                let reference = format!("{path}/{index}/{name}");
                let result = self
                    .run_task(name, task, &reference, current, token, variables)
                    .await;
                if result == Err(Halt::ContinueAsNew) {
                    self.suspend(Frame::Within { index });
                }
                let Ran { output, flow, .. } = result?;
                current = output;
                match flow {
                    Flow::Continue => index += 1,
                    Flow::Exit => break,
                    Flow::End => {
                        return Ok(Done {
                            output: current,
                            end: true,
                        });
                    }
                    Flow::Goto(target) => {
                        index = items
                            .iter()
                            .position(|(name, _)| **name == target)
                            .ok_or_else(|| {
                                ows::Error::runtime(
                                    "Unknown task",
                                    format!("then names {target}, which isn't in the same list."),
                                )
                                .at(&reference)
                            })?;
                    }
                }
            }
            Ok(Done {
                output: current,
                end: false,
            })
        }
        .boxed_local()
    }

    /// Runs one task: `if`, the task's records, `input.from`, the task itself,
    /// `output.as`, `export.as` and `then`.
    pub async fn run_task(
        &self,
        name: &str,
        definition: &Value,
        reference: &str,
        input: Value,
        token: &Token,
        variables: &[(String, Value)],
    ) -> Result<Ran, Halt> {
        // A resumed task carries on where it stopped, without checking `if` or
        // transforming its input again.
        let (input, transformed, started_at) =
            match self.resume_frame(|frame| matches!(frame, Frame::Task { .. })) {
                Some(Frame::Task {
                    input,
                    transformed,
                    started_at,
                }) => (input, Some(transformed), Some(started_at)),
                _ => (input, None, None),
            };
        let mut scope = self.scope();
        scope.variables = variables.to_vec();
        scope.task = json!({
            "name": name,
            "reference": reference,
            "definition": definition,
            "input": input,
        });
        if let Some(condition) = definition.get("if").filter(|_| transformed.is_none()) {
            match expr::evaluate_field(condition, &input, &scope) {
                Ok(value) if !expr::truthy(&value) => {
                    return Ok(Ran {
                        output: input,
                        flow: Flow::Continue,
                        skipped: true,
                    });
                }
                Ok(_) => {}
                Err(error) => {
                    let halt = Halt::Fault(error.at(reference));
                    let record = self.host.task_started(reference, name);
                    self.host.task_finished(record, &Err(halt.clone()));
                    return Err(halt);
                }
            }
        }
        let record = self.host.task_started(reference, name);
        scope.task["startedAt"] = started_at.unwrap_or_else(|| date_time(self.host.now()));
        let input = match transformed {
            Some(transformed) => Input::Transformed(transformed),
            None => Input::Raw(input),
        };
        let result = self
            .execute(name, definition, reference, record, input, scope, token)
            .await;
        // A task that continues as new carries on in the next run, which
        // records how it ends.
        if result != Err(Halt::ContinueAsNew) {
            let output = result
                .as_ref()
                .map(|(output, _)| output.clone())
                .map_err(Clone::clone);
            self.host.task_finished(record, &output);
        }
        let (output, flow) = result?;
        Ok(Ran {
            output,
            flow,
            skipped: false,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute(
        &self,
        name: &str,
        definition: &Value,
        reference: &str,
        record: usize,
        input: Input,
        mut scope: Scope,
        token: &Token,
    ) -> Result<(Value, Flow), Halt> {
        let input = match input {
            Input::Transformed(input) => input,
            Input::Raw(raw_input) => {
                schema::check(definition, "input", &raw_input).map_err(|e| e.at(reference))?;
                match definition.pointer("/input/from") {
                    Some(from) => expr::evaluate_field(from, &raw_input, &scope)
                        .map_err(|e| e.at(reference))?,
                    None => raw_input,
                }
            }
        };
        scope.input = input.clone();
        let after = timeout(definition, &input, &scope).map_err(|e| e.at(reference))?;
        let _blocked = (!tasks::resumable(definition)).then(|| self.block());
        let (outcome, mut scope) = self
            .within(after, token, "task", |token| async move {
                let task = Task {
                    name,
                    definition,
                    reference,
                    record,
                    input,
                    scope,
                    token: &token,
                };
                let outcome = tasks::execute(self, &task).await?;
                Ok((outcome, task.scope))
            })
            .await
            .map_err(|halt| halt.at(reference))?;

        scope.task["output"] = outcome.output.clone();
        let output = match definition.pointer("/output/as") {
            Some(output) => expr::evaluate_field(output, &outcome.output, &scope)
                .map_err(|e| e.at(reference))?,
            None => outcome.output,
        };
        schema::check(definition, "output", &output).map_err(|e| e.at(reference))?;
        if let Some(export) = definition.get("export") {
            let context = match export.get("as") {
                Some(export) => {
                    scope.output = output.clone();
                    expr::evaluate_field(export, &output, &scope).map_err(|e| e.at(reference))?
                }
                None => scope.context.clone(),
            };
            schema::check(definition, "export", &context).map_err(|e| e.at(reference))?;
            self.set_context(context);
        }

        let flow = outcome.flow.unwrap_or_else(|| {
            definition
                .get("then")
                .and_then(Value::as_str)
                .map_or(Flow::Continue, Flow::named)
        });
        Ok((output, flow))
    }
}

/// A task's input: raw, or already transformed by a run that continued as new.
enum Input {
    Raw(Value),
    Transformed(Value),
}

/// Unblocks continuing as new when dropped, as a task that blocked it ends.
struct Blocked<'a>(&'a Cell<usize>);

impl Drop for Blocked<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}
