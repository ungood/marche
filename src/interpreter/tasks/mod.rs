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

use super::{Halt, Host, Interpreter, Outcome, Task};
use crate::ows;

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
