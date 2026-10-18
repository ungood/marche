//! One module per task type. To add one, write its module, add it to
//! [`execute`], and list it in `SUPPORTED_TASKS` in `src/definition.rs` and
//! `SUPPORTED` in `tests/steps/ows.rs`.

mod do_list;
mod for_each;
mod fork;
mod raise;
mod run;
mod set;
mod switch;
mod try_catch;
mod wait;

use serde_json::Value;

use super::{Halt, Host, Interpreter, Outcome, Task};
use crate::ows;

/// Whether a run may continue as new inside this task: a `do` or a sequential
/// `for`, whose position is all the state it has. Anything else, including a
/// task with a `timeout`, has something in flight that a new run would lose.
pub fn resumable(definition: &Value) -> bool {
    if definition.get("timeout").is_some() {
        return false;
    }
    if definition.get("for").is_some() {
        return !for_each::is_parallel(definition);
    }
    let others = ["set", "raise", "switch", "fork", "run", "try", "wait"];
    definition.get("do").is_some() && others.iter().all(|kind| definition.get(kind).is_none())
}

/// Runs a task by its type, and returns its raw output.
pub async fn execute<H: Host>(
    interpreter: &Interpreter<'_, H>,
    task: &Task<'_>,
) -> Result<Outcome, Halt> {
    let definition = task.definition;
    if definition.get("set").is_some() {
        set::execute(task)
    } else if definition.get("raise").is_some() {
        raise::execute(task)
    } else if definition.get("switch").is_some() {
        switch::execute(task)
    } else if definition.get("fork").is_some() {
        fork::execute(interpreter, task).await
    } else if definition.get("run").is_some() {
        run::execute(interpreter, task).await
    } else if definition.get("try").is_some() {
        try_catch::execute(interpreter, task).await
    } else if definition.get("wait").is_some() {
        wait::execute(interpreter, task).await
    } else if definition.get("for").is_some() {
        for_each::execute(interpreter, task).await
    } else if definition.get("do").is_some() {
        // Checked last, because other task types, such as `for`, have a `do` too.
        do_list::execute(interpreter, task).await
    } else {
        Err(ows::Error::runtime(
            "Unsupported task",
            "marche doesn't support this task type yet.",
        )
        .into())
    }
}
