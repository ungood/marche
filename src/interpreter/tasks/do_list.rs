//! `do`: runs its tasks in order, following `then`, in a scope of its own.

use crate::interpreter::{Flow, Halt, Host, Interpreter, Outcome, Task};

pub async fn execute<H: Host>(
    interpreter: &Interpreter<'_, H>,
    task: &Task<'_>,
) -> Result<Outcome, Halt> {
    let token = task.token.child_token();
    let done = interpreter
        .run_list(
            &task.definition["do"],
            format!("{}/do", task.reference),
            task.input.clone(),
            &token,
        )
        .await;
    // Leaving the scope cancels anything still running in it.
    token.cancel();
    let done = done?;
    Ok(Outcome {
        output: done.output,
        flow: done.end.then_some(Flow::End),
    })
}
