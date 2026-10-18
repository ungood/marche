//! `fork`: runs its branches concurrently, each a task in a scope of its own
//! inside the fork's scope, and each on the fork's input. A branch's `then` is
//! ignored, since a branch has no list to continue in.
//!
//! The output is the array of the branches' outputs, in declared order. The
//! first branch to fault cancels the fork's scope, and the fork faults with its
//! error once the others have stopped. With `compete: true`, the first branch
//! to complete does the same, and its output is the fork's.
//!
//! "First" is the order in which the branch futures resolve inside
//! [`join_all`], which polls them in declared order as Temporal delivers each
//! activation, so it's the same on replay.

use std::cell::RefCell;

use serde_json::Value;

use crate::interpreter::{Halt, Host, Interpreter, Outcome, Task, join_all};

pub async fn execute<H: Host>(
    interpreter: &Interpreter<'_, H>,
    task: &Task<'_>,
) -> Result<Outcome, Halt> {
    let compete = task.definition.pointer("/fork/compete") == Some(&Value::Bool(true));
    let branches: Vec<(&String, &Value)> = task.definition["fork"]["branches"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|item| item.iter().next())
        .collect();
    let token = task.token.child_token();
    // The result of the first branch to settle the fork: the first to fault,
    // or with `compete`, the first to complete.
    let first: RefCell<Option<Result<Value, Halt>>> = RefCell::new(None);
    let runs = branches.iter().enumerate().map(|(index, (name, branch))| {
        let (token, first) = (&token, &first);
        async move {
            let reference = format!("{}/fork/branches/{index}/{name}", task.reference);
            let scope = token.child_token();
            let result = if scope.is_cancelled() {
                // An earlier branch settled the fork before this one started.
                let result = Err(Halt::Cancelled);
                interpreter.host.task_started(&reference, name);
                interpreter.host.task_finished(&reference, &result);
                result
            } else {
                interpreter
                    .run_task(name, branch, &reference, task.input.clone(), &scope)
                    .await
                    .map(|(output, _)| output)
            };
            let settles = match &result {
                Ok(_) => compete,
                Err(halt) => matches!(halt, Halt::Fault(_)),
            };
            if settles && first.borrow().is_none() {
                *first.borrow_mut() = Some(result.clone());
                token.cancel();
            }
            result
        }
    });
    let results = join_all(runs).await;
    // Leaving the scope cancels anything still running in it.
    token.cancel();
    if let Some(result) = first.into_inner() {
        return result.map(Outcome::output);
    }
    let outputs = results.into_iter().collect::<Result<Vec<_>, _>>()?;
    Ok(Outcome::output(Value::Array(outputs)))
}
