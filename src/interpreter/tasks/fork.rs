//! `fork`: runs its branches concurrently, each a task in a scope of its own
//! inside the fork's scope, and each on the fork's input. A branch's
//! `then: end` ends the workflow once the fork has finished. Its other `then`
//! values are ignored, since a branch has no list to continue in.
//!
//! The output is the array of the branches' outputs, in declared order. The
//! first branch to fault cancels the fork's scope, and the fork faults with its
//! error once the others have stopped. With `compete: true`, the first branch
//! to complete does the same, and its output is the fork's.
//!
//! A branch skipped by its `if` keeps its place in the array with its input,
//! as a skipped task's output is its input, but it never wins a compete. When
//! every branch is skipped, the fork's output is its input.
//!
//! "First" is the order in which the branch futures resolve inside
//! [`join_all`], which polls them in declared order as Temporal delivers each
//! activation, so it's the same on replay.

use std::cell::RefCell;

use serde_json::Value;

use crate::interpreter::{Flow, Halt, Host, Interpreter, Outcome, Ran, Task, join_all};

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
    let first: RefCell<Option<Result<Ran, Halt>>> = RefCell::new(None);
    let runs = branches.iter().enumerate().map(|(index, (name, branch))| {
        let (token, first) = (&token, &first);
        async move {
            let reference = format!("{}/fork/branches/{index}/{name}", task.reference);
            let scope = token.child_token();
            let result = if scope.is_cancelled() {
                // An earlier branch settled the fork before this one started.
                let record = interpreter.host.task_started(&reference, name);
                interpreter
                    .host
                    .task_finished(record, &Err(Halt::Cancelled));
                Err(Halt::Cancelled)
            } else {
                interpreter
                    .run_task(
                        name,
                        branch,
                        &reference,
                        task.input.clone(),
                        &scope,
                        &task.scope.variables,
                    )
                    .await
            };
            let settles = match &result {
                Ok(ran) => compete && !ran.skipped,
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
        return result.map(|ran| Outcome {
            flow: (ran.flow == Flow::End).then_some(Flow::End),
            output: ran.output,
        });
    }
    let branches = results.into_iter().collect::<Result<Vec<_>, _>>()?;
    if !branches.is_empty() && branches.iter().all(|ran| ran.skipped) {
        return Ok(Outcome::output(task.input.clone()));
    }
    Ok(Outcome {
        flow: branches
            .iter()
            .any(|ran| ran.flow == Flow::End)
            .then_some(Flow::End),
        output: Value::Array(branches.into_iter().map(|ran| ran.output).collect()),
    })
}
