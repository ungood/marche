//! `set`: the output is the task's value, with its expressions evaluated.

use crate::expr;
use crate::interpreter::{Halt, Outcome, Task};

pub fn execute(task: &Task<'_>) -> Result<Outcome, Halt> {
    let output = expr::evaluate_template(&task.definition["set"], &task.input, &task.scope)?;
    Ok(Outcome::output(output))
}
