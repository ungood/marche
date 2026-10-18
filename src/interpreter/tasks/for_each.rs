//! `for`: runs its tasks once for each item of a collection, each iteration in
//! a scope of its own. Iterations run one after another, each one's output the
//! next one's input, unless the task's `metadata.marche` makes them parallel.

use std::cell::{Cell, RefCell};

use serde_json::Value;

use crate::expr::{self, Scope};
use crate::interpreter::{Done, Flow, Halt, Host, Interpreter, Outcome, Task, Token, join_all};
use crate::ows;

pub async fn execute<H: Host>(
    interpreter: &Interpreter<'_, H>,
    task: &Task<'_>,
) -> Result<Outcome, Halt> {
    let items = items(task)?;
    if let Some(settings) = Parallel::of(task.definition) {
        return parallel(interpreter, task, &items, &settings).await;
    }
    let path = format!("{}/do", task.reference);
    let mut current = task.input.clone();
    for (index, item) in items.iter().enumerate() {
        let scope = iteration_scope(task, item, index);
        if let Some(condition) = task.definition.get("while")
            && !expr::truthy(&expr::evaluate_field(condition, &current, &scope)?)
        {
            break;
        }
        let token = task.token.child_token();
        let done = interpreter
            .run_list_with(
                &task.definition["do"],
                path.clone(),
                current,
                &token,
                &scope.variables,
            )
            .await;
        // Leaving the iteration's scope cancels anything still running in it.
        token.cancel();
        let done = done?;
        current = done.output;
        if done.end {
            return Ok(Outcome {
                output: current,
                flow: Some(Flow::End),
            });
        }
    }
    Ok(Outcome::output(current))
}

/// The collection `for.in` gives, on the task's input.
fn items(task: &Task<'_>) -> Result<Vec<Value>, ows::Error> {
    let collection = &task.definition["for"]["in"];
    match expr::evaluate_field(collection, &task.input, &task.scope)? {
        Value::Array(items) => Ok(items),
        other => Err(ows::Error::expression(format!(
            "for.in must give an array, but it gave {other}."
        ))),
    }
}

/// The task's scope with the item and its index bound, as `for.each` and
/// `for.at` name them.
fn iteration_scope(task: &Task<'_>, item: &Value, index: usize) -> Scope {
    let spec = &task.definition["for"];
    let each = spec.get("each").and_then(Value::as_str).unwrap_or("item");
    let at = spec.get("at").and_then(Value::as_str).unwrap_or("index");
    let mut scope = task.scope.clone();
    scope.bind(each, item.clone());
    scope.bind(at, Value::from(index));
    scope
}

/// The extension's settings under `metadata.marche`, for a parallel loop.
struct Parallel {
    max_concurrency: Option<usize>,
    cancel_on_failure: bool,
}

impl Parallel {
    /// The settings, or `None` when the loop runs sequentially.
    fn of(definition: &Value) -> Option<Self> {
        let settings = definition.pointer("/metadata/marche")?;
        if settings.get("parallel") != Some(&Value::Bool(true)) {
            return None;
        }
        Some(Self {
            max_concurrency: settings
                .get("maxConcurrency")
                .and_then(Value::as_u64)
                .map(|max| usize::try_from(max).unwrap_or(usize::MAX)),
            cancel_on_failure: settings
                .get("cancelOnFailure")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        })
    }
}

/// What the lanes of a parallel loop share.
struct Lanes {
    /// The index of the next item to start.
    next: Cell<usize>,
    /// Each iteration's result, by index, once it has finished.
    results: RefCell<Vec<Option<Result<Done, Halt>>>>,
    /// With `cancelOnFailure`, the error of the first iteration to fault.
    failure: RefCell<Option<ows::Error>>,
}

/// Runs the iterations concurrently, `maxConcurrency` at most. Each lane takes
/// the next item as soon as its last iteration finishes, so iterations start in
/// item order, and Temporal's `join_all` polls the lanes in a fixed order.
async fn parallel<H: Host>(
    interpreter: &Interpreter<'_, H>,
    task: &Task<'_>,
    items: &[Value],
    settings: &Parallel,
) -> Result<Outcome, Halt> {
    let lanes = Lanes {
        next: Cell::new(0),
        results: RefCell::new(items.iter().map(|_| None).collect()),
        failure: RefCell::new(None),
    };
    let count = settings
        .max_concurrency
        .unwrap_or(items.len())
        .clamp(1, items.len().max(1));
    let token = task.token.child_token();
    join_all((0..count).map(|_| lane(interpreter, task, items, settings, &token, &lanes))).await;
    // Leaving the scope cancels anything still running in it.
    token.cancel();
    if task.token.is_cancelled() {
        return Err(Halt::Cancelled);
    }
    if let Some(error) = lanes.failure.into_inner() {
        return Err(error.into());
    }
    let mut outputs = Vec::with_capacity(items.len());
    let mut end = false;
    // Without `cancelOnFailure`, the lowest-indexed fault is the task's error.
    for result in lanes.results.into_inner() {
        let done = result.unwrap_or(Err(Halt::Cancelled))?;
        end |= done.end;
        outputs.push(done.output);
    }
    Ok(Outcome {
        output: Value::Array(outputs),
        flow: end.then_some(Flow::End),
    })
}

/// Runs iterations one after another, taking the next item each time, until
/// none are left or the loop's scope is cancelled.
async fn lane<H: Host>(
    interpreter: &Interpreter<'_, H>,
    task: &Task<'_>,
    items: &[Value],
    settings: &Parallel,
    token: &Token,
    lanes: &Lanes,
) {
    let path = format!("{}/do", task.reference);
    while !token.is_cancelled() {
        let index = lanes.next.get();
        let Some(item) = items.get(index) else { return };
        lanes.next.set(index + 1);
        let scope = iteration_scope(task, item, index);
        let iteration = token.child_token();
        let result = interpreter
            .run_list_with(
                &task.definition["do"],
                path.clone(),
                task.input.clone(),
                &iteration,
                &scope.variables,
            )
            .await;
        iteration.cancel();
        if let Err(Halt::Fault(error)) = &result
            && settings.cancel_on_failure
            && lanes.failure.borrow().is_none()
        {
            *lanes.failure.borrow_mut() = Some(error.clone());
            // The statechart rule: the fault leaves the loop's scope, which
            // cancels the other iterations, and no more start.
            token.cancel();
        }
        lanes.results.borrow_mut()[index] = Some(result);
    }
}
