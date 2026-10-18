//! `raise`: faults with the error defined inline, after evaluating the `${ }`
//! expressions in its `type`, `title`, `detail` and `instance`. The task's
//! reference becomes the instance when the error doesn't give one.

use serde_json::Value;

use crate::expr;
use crate::interpreter::{Halt, Outcome, Task};
use crate::ows;

pub fn execute(task: &Task<'_>) -> Result<Outcome, Halt> {
    let (Ok(error) | Err(error)) = error(task);
    Err(error.into())
}

/// The error to raise, or the error evaluating it.
fn error(task: &Task<'_>) -> Result<ows::Error, ows::Error> {
    let Some(definition) = task
        .definition
        .pointer("/raise/error")
        .and_then(Value::as_object)
    else {
        return Err(ows::Error::runtime(
            "Unsupported error",
            "marche only raises errors defined inline.",
        ));
    };
    let text = |field: &str| -> Result<Option<String>, ows::Error> {
        let Some(value) = definition.get(field) else {
            return Ok(None);
        };
        match expr::evaluate_template(value, &task.input, &task.scope)? {
            Value::String(text) => Ok(Some(text)),
            other => Err(ows::Error::expression(format!(
                "The error's {field} must be a string, but it's {other}."
            ))),
        }
    };
    let status = definition.get("status").and_then(Value::as_u64);
    Ok(ows::Error {
        kind: text("type")?.unwrap_or_default(),
        status: status.and_then(|s| u16::try_from(s).ok()).unwrap_or(500),
        title: text("title")?,
        detail: text("detail")?,
        instance: text("instance")?,
    })
}
