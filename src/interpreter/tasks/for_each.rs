//! `for`: runs its tasks once for each item of a collection, each iteration in
//! a scope of its own. Iterations run one after another, each one's output the
//! next one's input, unless the task's `metadata.marche` makes them parallel.

use serde_json::Value;

use crate::expr::{self, Scope};
use crate::interpreter::{Flow, Halt, Host, Interpreter, Outcome, Task};
use crate::ows;

pub async fn execute<H: Host>(
    interpreter: &Interpreter<'_, H>,
    task: &Task<'_>,
) -> Result<Outcome, Halt> {
    let items = items(task)?;
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
