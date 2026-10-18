//! `switch`: follows the `then` of the first case whose `when` is true. A case
//! without `when` always matches. When none matches, the task's own `then`
//! applies. The output is the input.

use serde_json::Value;

use crate::expr;
use crate::interpreter::{Flow, Halt, Outcome, Task};

pub fn execute(task: &Task<'_>) -> Result<Outcome, Halt> {
    let cases = task.definition["switch"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|item| item.values().next());
    for case in cases {
        let matches = match case.get("when") {
            Some(when) => expr::truthy(&expr::evaluate_field(when, &task.input, &task.scope)?),
            None => true,
        };
        if matches {
            let then = case
                .get("then")
                .and_then(Value::as_str)
                .unwrap_or("continue");
            return Ok(Outcome {
                output: task.input.clone(),
                flow: Some(Flow::named(then)),
            });
        }
    }
    Ok(Outcome::output(task.input.clone()))
}
