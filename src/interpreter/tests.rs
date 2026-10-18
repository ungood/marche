//! Interpreter tests against a fake host.

use std::cell::RefCell;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::FutureExt;
use futures::future::LocalBoxFuture;
use serde_json::{Value, json};

use super::*;
use crate::shell::{ShellRequest, ShellResult};

/// A host that records task events and timers, and answers shell steps from
/// a script. Tests for new task types can extend it.
#[derive(Default)]
struct FakeHost {
    events: RefCell<Vec<String>>,
    requests: RefCell<Vec<ShellRequest>>,
    results: RefCell<Vec<ShellResult>>,
    timers: RefCell<Vec<Duration>>,
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

    fn shell(
        &self,
        request: ShellRequest,
        token: &Token,
    ) -> LocalBoxFuture<'_, Result<ShellResult, Halt>> {
        self.requests.borrow_mut().push(request);
        let result = if token.is_cancelled() {
            Err(Halt::Cancelled)
        } else {
            Ok(self.results.borrow_mut().remove(0))
        };
        async move { result }.boxed_local()
    }

    /// Timers fire at once, unless cancelled first.
    fn timer(&self, duration: Duration, token: &Token) -> LocalBoxFuture<'_, Result<(), Halt>> {
        self.timers.borrow_mut().push(duration);
        let result = if token.is_cancelled() {
            Err(Halt::Cancelled)
        } else {
            Ok(())
        };
        async move { result }.boxed_local()
    }

    fn random(&self, _stream: &str) -> f64 {
        0.5
    }

    fn continue_as_new_suggested(&self) -> bool {
        false
    }
}

fn run(host: &FakeHost, definition: &str, input: Value) -> Result<Value, Halt> {
    let definition: Value = serde_yaml_ng::from_str(definition).unwrap();
    let mut interpreter = Interpreter::new(host, definition, "test.run-1", "/work");
    futures::executor::block_on(interpreter.run(input, &Token::new()))
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
        include_str!("../../examples/hello.yaml"),
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

#[test]
fn a_cancelled_scope_starts_no_more_tasks() {
    let host = FakeHost::default();
    let definition: Value =
        serde_yaml_ng::from_str(include_str!("../../examples/hello.yaml")).unwrap();
    let mut interpreter = Interpreter::new(&host, definition, "test.run-1", "/work");
    let token = Token::new();
    token.cancel();
    let output = futures::executor::block_on(interpreter.run(json!({}), &token));
    assert_eq!(output, Err(Halt::Cancelled));
    assert!(host.events.borrow().is_empty());
}

#[test]
fn cancelling_a_parent_cancels_its_children() {
    let parent = Token::new();
    let child = parent.child_token();
    parent.cancel();
    assert!(child.is_cancelled());
}
