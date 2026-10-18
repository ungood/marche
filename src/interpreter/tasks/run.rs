//! `run: shell`: evaluates the step's expressions and runs it through the host.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};

use crate::expr;
use crate::interpreter::{Halt, Host, Interpreter, Outcome, Task};
use crate::ows;
use crate::shell::ShellRequest;

pub async fn execute<H: Host>(
    interpreter: &Interpreter<'_, H>,
    task: &Task<'_>,
) -> Result<Outcome, Halt> {
    let Some(shell) = task.definition.pointer("/run/shell") else {
        return Err(
            ows::Error::runtime("Unsupported task", "marche only runs shell processes.").into(),
        );
    };
    let (input, scope) = (&task.input, &task.scope);
    let text = |field: &str| -> Result<Option<String>, ows::Error> {
        shell
            .get(field)
            .map(|value| expr::evaluate_template(value, input, scope).map(|v| as_text(&v)))
            .transpose()
    };
    let command = text("command")?.unwrap_or_default();
    let arguments = match shell.get("arguments") {
        Some(arguments) => expr::evaluate_template(arguments, input, scope)?
            .as_array()
            .into_iter()
            .flatten()
            .map(as_text)
            .collect(),
        None => Vec::new(),
    };
    let environment: BTreeMap<String, String> = match shell.get("environment") {
        Some(environment) => expr::evaluate_template(environment, input, scope)?
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, value)| (name.clone(), as_text(value)))
            .collect(),
        None => BTreeMap::new(),
    };
    let directory = match text("directory")? {
        Some(directory) => Path::new(&interpreter.directory).join(directory),
        None => Path::new(&interpreter.directory).to_path_buf(),
    };
    let request = ShellRequest {
        reference: task.reference.to_string(),
        record: task.record,
        command,
        arguments,
        environment,
        stdin: text("stdin")?,
        directory: directory.to_string_lossy().into_owned(),
    };
    let result = interpreter.host.shell(request, task.token).await?;
    let returns = task
        .definition
        .pointer("/run/return")
        .and_then(Value::as_str)
        .unwrap_or("stdout");
    if result.code != 0 && !matches!(returns, "code" | "all") {
        return Err(ows::Error::runtime("Command failed", result.failure()).into());
    }
    Ok(Outcome::output(match returns {
        "stderr" => Value::String(result.stderr),
        "code" => Value::from(result.code),
        "all" => json!({"code": result.code, "stdout": result.stdout, "stderr": result.stderr}),
        "none" => Value::Null,
        _ => Value::String(result.stdout),
    }))
}

/// A value as a shell sees it: strings as they are, anything else as JSON.
fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
