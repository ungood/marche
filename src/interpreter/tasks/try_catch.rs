//! `try`: runs its tasks in a scope of their own, and handles the errors that
//! `catch` matches, by retrying the whole block or by running `catch.do`.
//! Cancellation is never caught.

use std::time::Duration;

use serde_json::Value;

use super::wait;
use crate::expr::{self, Scope};
use crate::interpreter::{Flow, Halt, Host, Interpreter, Outcome, Task};
use crate::ows;

/// The stream retry jitter draws from, so it doesn't shift other randomness.
const JITTER_STREAM: &str = "marche.jitter";

pub async fn execute<H: Host>(
    interpreter: &Interpreter<'_, H>,
    task: &Task<'_>,
) -> Result<Outcome, Halt> {
    let catch = &task.definition["catch"];
    let name = catch.get("as").and_then(Value::as_str).unwrap_or("error");
    let retry = catch.get("retry").filter(|retry| retry.is_object());
    let started = interpreter.host.now();
    let mut attempt = 1;
    let scope = loop {
        let token = task.token.child_token();
        let done = interpreter
            .run_list_with(
                &task.definition["try"],
                format!("{}/try", task.reference),
                task.input.clone(),
                &token,
                &task.scope.variables,
            )
            .await;
        // Leaving the scope cancels anything still running in it.
        token.cancel();
        let error = match done {
            Ok(done) => {
                return Ok(Outcome {
                    output: done.output,
                    flow: done.end.then_some(Flow::End),
                });
            }
            // A try blocks continuing as new, so only cancellation gets here.
            Err(halt @ (Halt::Cancelled | Halt::ContinueAsNew)) => return Err(halt),
            Err(Halt::Fault(error)) => error,
        };
        let mut scope = task.scope.clone();
        // The tasks that ran may have exported.
        scope.context = interpreter.context();
        scope.bind(
            name,
            serde_json::to_value(&error).expect("errors serialize"),
        );
        if !catches(catch, &error, &task.input, &scope)? {
            return Err(error.into());
        }
        let Some(policy) = retry else { break scope };
        let elapsed = interpreter
            .host
            .now()
            .duration_since(started)
            .unwrap_or_default();
        let random = || interpreter.host.random(JITTER_STREAM);
        match next_delay(policy, attempt, elapsed, &task.input, &scope, random)? {
            Some(delay) => interpreter.host.timer(delay, task.token).await?,
            // Out of retries: `catch.do` handles the last error, if there is one.
            None if catch.get("do").is_some() => break scope,
            None => return Err(error.into()),
        }
        attempt += 1;
    };

    let then = catch.get("then").and_then(Value::as_str).map(Flow::named);
    let Some(tasks) = catch.get("do") else {
        return Ok(Outcome {
            output: task.input.clone(),
            flow: then,
        });
    };
    let token = task.token.child_token();
    let done = interpreter
        .run_list_with(
            tasks,
            format!("{}/catch/do", task.reference),
            task.input.clone(),
            &token,
            &scope.variables,
        )
        .await;
    token.cancel();
    let done = done?;
    Ok(Outcome {
        output: done.output,
        flow: if done.end { Some(Flow::End) } else { then },
    })
}

/// Whether `catch` handles the error: it has every field in `errors.with`,
/// `when` holds and `exceptWhen` doesn't.
fn catches(
    catch: &Value,
    error: &ows::Error,
    input: &Value,
    scope: &Scope,
) -> Result<bool, ows::Error> {
    let fields = serde_json::to_value(error).expect("errors serialize");
    let matches = catch
        .pointer("/errors/with")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .all(|(field, value)| fields.get(field) == Some(value));
    Ok(matches && conditions(catch, input, scope)?)
}

/// Whether `when`, if set, holds and `exceptWhen`, if set, doesn't.
fn conditions(object: &Value, input: &Value, scope: &Scope) -> Result<bool, ows::Error> {
    if let Some(when) = object.get("when")
        && !expr::truthy(&expr::evaluate_field(when, input, scope)?)
    {
        return Ok(false);
    }
    if let Some(except) = object.get("exceptWhen")
        && expr::truthy(&expr::evaluate_field(except, input, scope)?)
    {
        return Ok(false);
    }
    Ok(true)
}

/// The wait before the attempt after `attempt`, counting from 1, or `None` when
/// the policy doesn't retry again. `elapsed` is the time since the first attempt
/// started, and `random` draws a number in [0, 1) for jitter.
fn next_delay(
    policy: &Value,
    attempt: u32,
    elapsed: Duration,
    input: &Value,
    scope: &Scope,
    random: impl FnOnce() -> f64,
) -> Result<Option<Duration>, ows::Error> {
    if !conditions(policy, input, scope)? {
        return Ok(None);
    }
    if let Some(count) = policy
        .pointer("/limit/attempt/count")
        .and_then(Value::as_u64)
        && u64::from(attempt) >= count
    {
        return Ok(None);
    }
    let duration = |pointer: &str| {
        policy
            .pointer(pointer)
            .map(|value| wait::evaluate(value, input, scope))
            .transpose()
    };
    let base = duration("/delay")?.unwrap_or_default();
    let backoff = policy.get("backoff");
    let mut delay = if backoff.is_some_and(|b| b.get("linear").is_some()) {
        base.saturating_mul(attempt)
    } else if backoff.is_some_and(|b| b.get("exponential").is_some()) {
        base.saturating_mul(2u32.saturating_pow(attempt - 1))
    } else {
        base
    };
    if let (Some(from), Some(to)) = (duration("/jitter/from")?, duration("/jitter/to")?) {
        let spread = to.saturating_sub(from).mul_f64(random());
        delay = delay.saturating_add(from).saturating_add(spread);
    }
    let delay = delay.min(crate::interpreter::duration::MAX);
    if let Some(limit) = duration("/limit/duration")?
        && elapsed.saturating_add(delay) > limit
    {
        return Ok(None);
    }
    Ok(Some(delay))
}
