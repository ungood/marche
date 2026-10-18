//! `wait`: pauses on a durable timer. The output is the task's input.

use std::time::Duration;

use serde_json::Value;

use crate::expr::{self, Scope};
use crate::interpreter::{Halt, Host, Interpreter, Outcome, Task, duration};
use crate::ows;

pub async fn execute<H: Host>(
    interpreter: &Interpreter<'_, H>,
    task: &Task<'_>,
) -> Result<Outcome, Halt> {
    let wait = evaluate(&task.definition["wait"], &task.input, &task.scope)?;
    interpreter.host.timer(wait, task.token).await?;
    Ok(Outcome::output(task.input.clone()))
}

/// A duration as a definition writes it, which may also be an expression that
/// gives an ISO 8601 duration.
pub fn evaluate(value: &Value, input: &Value, scope: &Scope) -> Result<Duration, ows::Error> {
    duration::parse(&expr::evaluate_template(value, input, scope)?)
}
