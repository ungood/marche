//! The OWS interpreter. It walks a definition's task tree and evaluates its
//! expressions, and leaves everything that touches the outside world to a
//! [`Host`]: in a run, the Temporal workflow `marche.interpret`.
//!
//! This code runs as workflow code, so it must be deterministic: no clocks,
//! randomness, environment, I/O or unordered iteration except through the host.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::FutureExt;
use futures::future::LocalBoxFuture;
use serde_json::{Value, json};

use crate::expr::{self, Scope};
use crate::ows;
use crate::shell::{ShellRequest, ShellResult};

/// Why a task or run stopped without an output.
#[derive(Debug, Clone, PartialEq)]
pub enum Halt {
    Fault(ows::Error),
    Cancelled,
}

impl From<ows::Error> for Halt {
    fn from(error: ows::Error) -> Self {
        Self::Fault(error)
    }
}

impl Halt {
    fn at(self, reference: &str) -> Self {
        match self {
            Self::Fault(error) => Self::Fault(error.at(reference)),
            Self::Cancelled => Self::Cancelled,
        }
    }
}

/// What the interpreter needs from the workflow that runs it.
pub trait Host {
    /// The workflow clock, which replays identically.
    fn now(&self) -> SystemTime;
    fn task_started(&self, reference: &str, name: &str);
    fn task_finished(&self, reference: &str, result: &Result<Value, Halt>);
    fn shell(&self, request: ShellRequest) -> LocalBoxFuture<'_, Result<ShellResult, Halt>>;
}

/// Where a run goes after a task, from its `then`.
enum Flow {
    Continue,
    Exit,
    End,
    Goto(String),
}

/// The result of a task list: its output, and whether `then: end` finished the
/// whole workflow.
struct Done {
    output: Value,
    end: bool,
}

pub struct Interpreter<'h, H> {
    host: &'h H,
    definition: Value,
    workflow: Value,
    runtime: Value,
    directory: String,
    context: RefCell<Value>,
}

/// The spec's dateTimeDescriptor.
fn date_time(time: SystemTime) -> Value {
    let since = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let millis = since.as_millis() as i64;
    let iso8601 = jiff::Timestamp::from_millisecond(millis)
        .map(|t| t.to_string())
        .unwrap_or_default();
    json!({
        "iso8601": iso8601,
        "epoch": {"seconds": since.as_secs(), "milliseconds": millis},
    })
}

impl<'h, H: Host> Interpreter<'h, H> {
    /// `id` is the run's ID, and `directory` is where `marche run` was started.
    pub fn new(host: &'h H, definition: Value, id: &str, directory: &str) -> Self {
        Self {
            host,
            workflow: json!({"id": id, "definition": definition}),
            runtime: json!({
                "name": "marche",
                "version": env!("CARGO_PKG_VERSION"),
                "metadata": {"directory": directory},
            }),
            definition,
            directory: directory.to_string(),
            context: RefCell::new(json!({})),
        }
    }

    fn scope(&self) -> Scope {
        Scope {
            context: self.context.borrow().clone(),
            workflow: self.workflow.clone(),
            runtime: self.runtime.clone(),
            ..Scope::default()
        }
    }

    /// Runs the workflow on its raw input and returns its transformed output.
    pub async fn run(&mut self, input: Value) -> Result<Value, Halt> {
        self.workflow["input"] = input.clone();
        self.workflow["startedAt"] = date_time(self.host.now());
        let scope = self.scope();
        let definition = self.definition.clone();
        let input = match definition.pointer("/input/from") {
            Some(from) => expr::evaluate_field(from, &input, &scope).map_err(|e| e.at("/input"))?,
            None => input,
        };
        let done = self
            .run_list(&definition["do"], "/do".into(), input)
            .await?;
        match definition.pointer("/output/as") {
            Some(output) => {
                let scope = self.scope();
                Ok(expr::evaluate_field(output, &done.output, &scope)
                    .map_err(|e| e.at("/output"))?)
            }
            None => Ok(done.output),
        }
    }

    fn run_list<'a>(
        &'a self,
        list: &'a Value,
        path: String,
        input: Value,
    ) -> LocalBoxFuture<'a, Result<Done, Halt>> {
        async move {
            let items: Vec<(&String, &Value)> = list
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
                .filter_map(|item| item.iter().next())
                .collect();
            let mut index = 0;
            let mut current = input;
            while let Some((name, task)) = items.get(index) {
                let reference = format!("{path}/{index}/{name}");
                let (output, flow) = self.run_task(name, task, &reference, current).await?;
                current = output;
                match flow {
                    Flow::Continue => index += 1,
                    Flow::Exit => break,
                    Flow::End => {
                        return Ok(Done {
                            output: current,
                            end: true,
                        });
                    }
                    Flow::Goto(target) => {
                        index = items
                            .iter()
                            .position(|(name, _)| **name == target)
                            .ok_or_else(|| {
                                ows::Error::runtime(
                                    "Unknown task",
                                    format!("then names {target}, which isn't in the same list."),
                                )
                                .at(&reference)
                            })?;
                    }
                }
            }
            Ok(Done {
                output: current,
                end: false,
            })
        }
        .boxed_local()
    }

    async fn run_task(
        &self,
        name: &str,
        task: &Value,
        reference: &str,
        input: Value,
    ) -> Result<(Value, Flow), Halt> {
        let mut scope = self.scope();
        scope.task = json!({
            "name": name,
            "reference": reference,
            "definition": task,
            "input": input,
        });
        if let Some(condition) = task.get("if") {
            match expr::evaluate_field(condition, &input, &scope) {
                Ok(value) if !expr::truthy(&value) => return Ok((input, Flow::Continue)),
                Ok(_) => {}
                Err(error) => {
                    let result = Err(Halt::Fault(error.at(reference)));
                    self.host.task_started(reference, name);
                    self.host.task_finished(reference, &result);
                    return result.map(|output| (output, Flow::Continue));
                }
            }
        }
        self.host.task_started(reference, name);
        scope.task["startedAt"] = date_time(self.host.now());
        let result = self.execute(task, reference, input, scope).await;
        let output = result
            .as_ref()
            .map(|(output, _)| output.clone())
            .map_err(Clone::clone);
        self.host.task_finished(reference, &output);
        result
    }

    async fn execute(
        &self,
        task: &Value,
        reference: &str,
        raw_input: Value,
        mut scope: Scope,
    ) -> Result<(Value, Flow), Halt> {
        let input = match task.pointer("/input/from") {
            Some(from) => {
                expr::evaluate_field(from, &raw_input, &scope).map_err(|e| e.at(reference))?
            }
            None => raw_input,
        };
        scope.input = input.clone();
        let mut end = false;
        let raw_output = if let Some(set) = task.get("set") {
            expr::evaluate_template(set, &input, &scope).map_err(|e| e.at(reference))?
        } else if let Some(list) = task.get("do") {
            let done = self
                .run_list(list, format!("{reference}/do"), input.clone())
                .await?;
            end = done.end;
            done.output
        } else if let Some(shell) = task.pointer("/run/shell") {
            let returns = task.pointer("/run/return").and_then(Value::as_str);
            self.run_shell(shell, returns, reference, &input, &scope)
                .await
                .map_err(|halt| halt.at(reference))?
        } else {
            return Err(ows::Error::runtime(
                "Unsupported task",
                "marche doesn't support this task type yet.",
            )
            .at(reference)
            .into());
        };

        scope.task["output"] = raw_output.clone();
        let output = match task.pointer("/output/as") {
            Some(output) => {
                expr::evaluate_field(output, &raw_output, &scope).map_err(|e| e.at(reference))?
            }
            None => raw_output,
        };
        if let Some(export) = task.pointer("/export/as") {
            scope.output = output.clone();
            let context =
                expr::evaluate_field(export, &output, &scope).map_err(|e| e.at(reference))?;
            *self.context.borrow_mut() = context;
        }

        let flow = match (end, task.get("then").and_then(Value::as_str)) {
            (true, _) | (false, Some("end")) => Flow::End,
            (false, None | Some("continue")) => Flow::Continue,
            (false, Some("exit")) => Flow::Exit,
            (false, Some(target)) => Flow::Goto(target.to_string()),
        };
        Ok((output, flow))
    }

    async fn run_shell(
        &self,
        shell: &Value,
        returns: Option<&str>,
        reference: &str,
        input: &Value,
        scope: &Scope,
    ) -> Result<Value, Halt> {
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
            Some(directory) => Path::new(&self.directory).join(directory),
            None => Path::new(&self.directory).to_path_buf(),
        };
        let request = ShellRequest {
            reference: reference.to_string(),
            command,
            arguments,
            environment,
            stdin: text("stdin")?,
            directory: directory.to_string_lossy().into_owned(),
        };
        let result = self.host.shell(request).await?;
        let returns = returns.unwrap_or("stdout");
        if result.code != 0 && !matches!(returns, "code" | "all") {
            return Err(ows::Error::runtime("Command failed", result.failure()).into());
        }
        Ok(match returns {
            "stderr" => Value::String(result.stderr),
            "code" => Value::from(result.code),
            "all" => json!({"code": result.code, "stdout": result.stdout, "stderr": result.stderr}),
            "none" => Value::Null,
            _ => Value::String(result.stdout),
        })
    }
}

/// A value as a shell sees it: strings as they are, anything else as JSON.
fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use super::*;

    /// A host that records task events and answers shell steps from a script.
    #[derive(Default)]
    struct FakeHost {
        events: RefCell<Vec<String>>,
        requests: RefCell<Vec<ShellRequest>>,
        results: RefCell<Vec<ShellResult>>,
    }

    impl Host for FakeHost {
        fn now(&self) -> SystemTime {
            UNIX_EPOCH + Duration::from_secs(1_700_000_000)
        }

        fn task_started(&self, reference: &str, _name: &str) {
            self.events
                .borrow_mut()
                .push(format!("started {reference}"));
        }

        fn task_finished(&self, reference: &str, result: &Result<Value, Halt>) {
            let status = if result.is_ok() {
                "completed"
            } else {
                "faulted"
            };
            self.events
                .borrow_mut()
                .push(format!("{status} {reference}"));
        }

        fn shell(&self, request: ShellRequest) -> LocalBoxFuture<'_, Result<ShellResult, Halt>> {
            self.requests.borrow_mut().push(request);
            let result = self.results.borrow_mut().remove(0);
            async move { Ok(result) }.boxed_local()
        }
    }

    fn run(host: &FakeHost, definition: &str, input: Value) -> Result<Value, Halt> {
        let definition: Value = serde_yaml_ng::from_str(definition).unwrap();
        let mut interpreter = Interpreter::new(host, definition, "test.run-1", "/work");
        futures::executor::block_on(interpreter.run(input))
    }

    fn exited(code: i32, stdout: &str, stderr: &str) -> ShellResult {
        ShellResult {
            code,
            stdout: stdout.into(),
            stderr: stderr.into(),
            log: None,
        }
    }

    #[test]
    fn hello_runs_set_then_shell() {
        let host = FakeHost::default();
        host.results.borrow_mut().push(exited(0, "Hello, Ada", ""));
        let output = run(
            &host,
            include_str!("../examples/hello.yaml"),
            json!({"name": "Ada"}),
        );
        assert_eq!(output, Ok(json!("Hello, Ada")));
        let requests = host.requests.borrow();
        assert_eq!(requests[0].command, "echo \"Hello, $1\"");
        assert_eq!(requests[0].arguments, ["Ada"]);
        assert_eq!(requests[0].directory, "/work");
        assert_eq!(requests[0].reference, "/do/1/say");
        assert_eq!(
            *host.events.borrow(),
            [
                "started /do/0/greet",
                "completed /do/0/greet",
                "started /do/1/say",
                "completed /do/1/say"
            ]
        );
    }

    #[test]
    fn then_jumps_to_a_named_task() {
        let host = FakeHost::default();
        let output = run(
            &host,
            r#"
do:
  - setRed:
      set:
        colors: '${ .colors + [ "red" ] }'
      then: setGreen
  - setBlue:
      set:
        colors: '${ .colors + [ "blue" ] }'
      then: end
  - setGreen:
      set:
        colors: '${ .colors + [ "green" ] }'
      then: setBlue
"#,
            json!({}),
        );
        assert_eq!(output, Ok(json!({"colors": ["red", "green", "blue"]})));
    }

    #[test]
    fn a_failed_command_faults_the_task_and_its_parents() {
        let host = FakeHost::default();
        host.results.borrow_mut().push(exited(3, "", "boom\n"));
        let output = run(
            &host,
            r#"
do:
  - outer:
      do:
        - check:
            run:
              shell:
                command: exit 3
"#,
            json!({}),
        );
        let Err(Halt::Fault(error)) = output else {
            panic!("expected a fault: {output:?}")
        };
        assert_eq!(error.instance.as_deref(), Some("/do/0/outer/do/0/check"));
        assert_eq!(error.status, 500);
        assert_eq!(
            *host.events.borrow(),
            [
                "started /do/0/outer",
                "started /do/0/outer/do/0/check",
                "faulted /do/0/outer/do/0/check",
                "faulted /do/0/outer"
            ]
        );
    }

    #[test]
    fn relative_directories_resolve_against_the_invoking_directory() {
        let host = FakeHost::default();
        host.results.borrow_mut().push(exited(0, "", ""));
        host.results.borrow_mut().push(exited(0, "", ""));
        run(
            &host,
            r#"
do:
  - relative:
      run:
        shell:
          command: pwd
          directory: kitchen
  - absolute:
      run:
        shell:
          command: pwd
          directory: /tmp
"#,
            json!({}),
        )
        .unwrap();
        let requests = host.requests.borrow();
        assert_eq!(requests[0].directory, "/work/kitchen");
        assert_eq!(requests[1].directory, "/tmp");
    }

    #[test]
    fn started_at_comes_from_the_host_clock() {
        let host = FakeHost::default();
        let output = run(
            &host,
            r#"
do:
  - stamp:
      set:
        at: ${ $task.startedAt.iso8601 }
        epoch: ${ $workflow.startedAt.epoch.seconds }
"#,
            json!({}),
        );
        assert_eq!(
            output,
            Ok(json!({"at": "2023-11-14T22:13:20Z", "epoch": 1_700_000_000}))
        );
    }
}
