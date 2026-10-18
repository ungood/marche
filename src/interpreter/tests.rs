//! Interpreter tests against a fake host.

mod try_catch;
mod wait;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::task::Poll;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::FutureExt;
use futures::future::LocalBoxFuture;
use serde_json::{Value, json};

use super::*;
use crate::shell::{ShellRequest, ShellResult};

/// A host that records task events, timers and random draws, and answers shell
/// steps from a script. Its clock moves on only when a timer fires. A step with
/// nothing left in the script runs until it's cancelled. Tests for new task
/// types can extend it.
#[derive(Default)]
struct FakeHost {
    events: RefCell<Vec<String>>,
    requests: RefCell<Vec<ShellRequest>>,
    results: RefCell<Vec<ShellResult>>,
    timers: RefCell<Vec<Duration>>,
    streams: RefCell<Vec<String>>,
    /// How long shell steps take, by command, for steps that run concurrently:
    /// `Some(n)` finishes after yielding `n` times, and `None` runs until it's
    /// cancelled. Other steps finish at once.
    steps: RefCell<BTreeMap<String, Option<usize>>>,
}

impl Host for FakeHost {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_700_000_000) + self.timers.borrow().iter().sum()
    }

    fn task_started(&self, reference: &str, _name: &str) {
        self.events
            .borrow_mut()
            .push(format!("started {reference}"));
    }

    fn task_finished(&self, reference: &str, result: &Result<Value, Halt>) {
        let status = match result {
            Ok(_) => "completed",
            Err(Halt::Fault(_)) => "faulted",
            Err(Halt::Cancelled) => "cancelled",
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
        let step = self.steps.borrow().get(&request.command).copied();
        self.requests.borrow_mut().push(request);
        if token.is_cancelled() {
            return async { Err(Halt::Cancelled) }.boxed_local();
        }
        if let Some(step) = step {
            let token = token.clone();
            let result = step.map(|_| self.results.borrow_mut().remove(0));
            return async move {
                let Some(yields) = step else {
                    token.cancelled().await;
                    return Err(Halt::Cancelled);
                };
                for _ in 0..yields {
                    yield_now().await;
                    if token.is_cancelled() {
                        return Err(Halt::Cancelled);
                    }
                }
                Ok(result.expect("a result for the step"))
            }
            .boxed_local();
        }
        if self.results.borrow().is_empty() {
            let token = token.clone();
            return async move {
                token.cancelled().await;
                Err(Halt::Cancelled)
            }
            .boxed_local();
        }
        let result = Ok(self.results.borrow_mut().remove(0));
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

    fn random(&self, stream: &str) -> f64 {
        self.streams.borrow_mut().push(stream.to_string());
        0.5
    }

    fn continue_as_new_suggested(&self) -> bool {
        false
    }
}

/// Lets other futures run once, as a step that takes a while does.
async fn yield_now() {
    let mut yielded = false;
    futures::future::poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
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

const COLORS: &str = r#"
do:
  - switchColor:
      switch:
        - red:
            when: .color == "red"
            then: setRed
        - warm:
            when: .color == "red" or .color == "orange"
            then: setWarm
        - other:
            when: .color == "green"
            then: exit
      then: setNone
  - setRed:
      set:
        paint: red
      then: end
  - setWarm:
      set:
        paint: warm
      then: end
  - setNone:
      set:
        paint: none
"#;

#[test]
fn switch_follows_the_first_matching_case() {
    let host = FakeHost::default();
    let output = run(&host, COLORS, json!({"color": "red"}));
    assert_eq!(output, Ok(json!({"paint": "red"})));
    let output = run(&host, COLORS, json!({"color": "orange"}));
    assert_eq!(output, Ok(json!({"paint": "warm"})));
}

#[test]
fn switch_case_flow_directives_apply() {
    let host = FakeHost::default();
    let output = run(&host, COLORS, json!({"color": "green"}));
    assert_eq!(output, Ok(json!({"color": "green"})));
}

#[test]
fn switch_follows_its_own_then_when_no_case_matches() {
    let host = FakeHost::default();
    let output = run(&host, COLORS, json!({"color": "blue"}));
    assert_eq!(output, Ok(json!({"paint": "none"})));
    assert_eq!(
        *host.events.borrow(),
        [
            "started /do/0/switchColor",
            "completed /do/0/switchColor",
            "started /do/3/setNone",
            "completed /do/3/setNone"
        ]
    );
}

#[test]
fn a_switch_case_without_when_is_the_default() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - pick:
      switch:
        - never:
            when: "false"
            then: end
        - fallback:
            then: chosen
  - skipped:
      set:
        chosen: false
  - chosen:
      set:
        chosen: true
"#,
        json!({}),
    );
    assert_eq!(output, Ok(json!({"chosen": true})));
}

#[test]
fn a_failing_switch_case_faults_the_switch() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - pick:
      switch:
        - broken:
            when: .color | ascii_downcase
            then: end
"#,
        json!({"color": 7}),
    );
    let Err(Halt::Fault(error)) = output else {
        panic!("expected a fault: {output:?}")
    };
    assert_eq!(error.status, 400);
    assert_eq!(error.instance.as_deref(), Some("/do/0/pick"));
}

#[test]
fn raise_faults_with_its_error_at_its_reference() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - refuse:
      raise:
        error:
          type: ${ "https://example.com/errors/" + .reason }
          status: 409
          title: ${ "No " + .dish }
          detail: The kitchen ran out.
  - never:
      set: {}
"#,
        json!({"dish": "soup", "reason": "sold-out"}),
    );
    assert_eq!(
        output,
        Err(Halt::Fault(ows::Error {
            kind: "https://example.com/errors/sold-out".into(),
            status: 409,
            title: Some("No soup".into()),
            detail: Some("The kitchen ran out.".into()),
            instance: Some("/do/0/refuse".into()),
        }))
    );
    assert_eq!(
        *host.events.borrow(),
        ["started /do/0/refuse", "faulted /do/0/refuse"]
    );
}

#[test]
fn raise_keeps_an_instance_it_gives() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - refuse:
      raise:
        error:
          type: https://example.com/errors/sold-out
          status: 409
          instance: ${ "/kitchen/" + .dish }
"#,
        json!({"dish": "soup"}),
    );
    let Err(Halt::Fault(error)) = output else {
        panic!("expected a fault: {output:?}")
    };
    assert_eq!(error.instance.as_deref(), Some("/kitchen/soup"));
    assert_eq!(error.title, None);
}

#[test]
fn raise_fields_must_be_strings() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - refuse:
      raise:
        error:
          type: https://example.com/errors/sold-out
          status: 409
          title: ${ .count }
"#,
        json!({"count": 3}),
    );
    let Err(Halt::Fault(error)) = output else {
        panic!("expected a fault: {output:?}")
    };
    assert_eq!(error.status, 400);
    assert_eq!(
        error.detail.as_deref(),
        Some("The error's title must be a string, but it's 3.")
    );
}

#[test]
fn review_loop_gives_up_after_three_attempts() {
    let host = FakeHost::default();
    for code in [1, 0, 1, 0, 1] {
        host.results.borrow_mut().push(exited(code, "", ""));
    }
    let output = run(
        &host,
        include_str!("../../examples/review-loop.yaml"),
        json!({}),
    );
    let Err(Halt::Fault(error)) = output else {
        panic!("expected a fault: {output:?}")
    };
    assert_eq!(error.status, 422);
    assert_eq!(error.instance.as_deref(), Some("/do/4/giveUp"));
    let commands: Vec<String> = host
        .requests
        .borrow()
        .iter()
        .map(|request| request.command.clone())
        .collect();
    assert_eq!(
        commands,
        [
            "./review.sh",
            "./fix.sh",
            "./review.sh",
            "./fix.sh",
            "./review.sh"
        ]
    );
}

fn fault(output: Result<Value, Halt>) -> ows::Error {
    match output {
        Err(Halt::Fault(error)) => error,
        other => panic!("expected a fault: {other:?}"),
    }
}

#[test]
fn a_task_past_its_timeout_is_cancelled_and_faults() {
    let host = FakeHost::default();
    let error = fault(run(
        &host,
        r#"
do:
  - course:
      timeout:
        after:
          seconds: 2
      do:
        - nap:
            run:
              shell:
                command: sleep 60
"#,
        json!({}),
    ));
    assert_eq!(error.status, 408);
    assert_eq!(error.instance.as_deref(), Some("/do/0/course"));
    assert_eq!(
        error.detail.as_deref(),
        Some("The task didn't finish within 2s.")
    );
    assert_eq!(*host.timers.borrow(), [Duration::from_secs(2)]);
    assert_eq!(
        *host.events.borrow(),
        [
            "started /do/0/course",
            "started /do/0/course/do/0/nap",
            "cancelled /do/0/course/do/0/nap",
            "faulted /do/0/course"
        ]
    );
}

#[test]
fn review_loop_ends_when_the_review_passes() {
    let host = FakeHost::default();
    for code in [1, 0, 0] {
        host.results.borrow_mut().push(exited(code, "", ""));
    }
    let output = run(
        &host,
        include_str!("../../examples/review-loop.yaml"),
        json!({}),
    );
    assert_eq!(output, Ok(json!({"attempt": 2, "passed": true})));
}

#[test]
fn a_task_that_finishes_first_isnt_timed_out() {
    let host = FakeHost::default();
    host.results.borrow_mut().push(exited(0, "done", ""));
    let output = run(
        &host,
        r#"
do:
  - quick:
      timeout:
        after: ${ .after }
      run:
        shell:
          command: echo done
"#,
        json!({"after": "PT1M"}),
    );
    assert_eq!(output, Ok(json!("done")));
    assert_eq!(*host.timers.borrow(), [Duration::from_secs(60)]);
}

#[test]
fn a_workflow_past_its_timeout_faults() {
    let host = FakeHost::default();
    let error = fault(run(
        &host,
        r#"
timeout:
  after:
    milliseconds: 1500
do:
  - nap:
      run:
        shell:
          command: sleep 60
"#,
        json!({}),
    ));
    assert_eq!(error.status, 408);
    assert_eq!(error.instance.as_deref(), Some("/"));
    assert_eq!(
        error.detail.as_deref(),
        Some("The workflow didn't finish within 1.5s.")
    );
    assert_eq!(
        *host.events.borrow(),
        ["started /do/0/nap", "cancelled /do/0/nap"]
    );
}

#[test]
fn schemas_check_input_before_from_and_output_after_as() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
input:
  schema:
    document:
      required: [ order ]
output:
  as: .cooked
  schema:
    document:
      type: string
do:
  - cook:
      input:
        schema:
          document:
            required: [ order ]
        from: .order
      set:
        cooked: ${ .dish }
      output:
        as: .
        schema:
          document:
            required: [ cooked ]
      export:
        as: '{ cooked: true }'
        schema:
          document:
            properties:
              cooked:
                const: true
"#,
        json!({"order": {"dish": "soup"}}),
    );
    assert_eq!(output, Ok(json!("soup")));
}

#[test]
fn data_that_doesnt_match_its_schema_faults_with_a_validation_error() {
    for (definition, instance, detail) in [
        (
            "input: {schema: {document: {type: array}}}\ndo: [{cook: {set: {}}}]",
            "/input",
            "The input doesn't match input.schema: {} is not of type \"array\".",
        ),
        (
            "output: {schema: {document: {type: array}}}\ndo: [{cook: {set: {}}}]",
            "/output",
            "The output doesn't match output.schema: {} is not of type \"array\".",
        ),
        (
            "do: [{cook: {set: {}, output: {schema: {document: {type: array}}}}}]",
            "/do/0/cook",
            "The output doesn't match output.schema: {} is not of type \"array\".",
        ),
        (
            "do: [{cook: {set: {}, export: {as: '[]', schema: {document: {type: object}}}}}]",
            "/do/0/cook",
            "The context doesn't match export.schema: [] is not of type \"object\".",
        ),
    ] {
        let error = fault(run(&FakeHost::default(), definition, json!({})));
        assert_eq!(
            error.kind,
            "https://open-workflow-specification.org/spec/1.0.0/errors/validation"
        );
        assert_eq!(error.instance.as_deref(), Some(instance), "{definition}");
        assert_eq!(error.detail.as_deref(), Some(detail), "{definition}");
    }
}

/// A fork of shell steps, each branch named after its command.
fn fork(compete: bool, commands: &[&str]) -> String {
    let branches: String = commands
        .iter()
        .map(|command| {
            format!(
                "          - {command}:\n              run:\n                shell:\n                  command: {command}\n"
            )
        })
        .collect();
    format!(
        "do:\n  - cook:\n      fork:\n        compete: {compete}\n        branches:\n{branches}"
    )
}

/// Makes each command a step that takes that many yields, or runs until cancelled.
fn steps(host: &FakeHost, steps: &[(&str, Option<usize>)]) {
    for (command, step) in steps {
        host.steps.borrow_mut().insert(command.to_string(), *step);
    }
}

#[test]
fn fork_output_keeps_declared_order() {
    let host = FakeHost::default();
    steps(&host, &[("sauce", Some(3)), ("rice", Some(1))]);
    host.results.borrow_mut().push(exited(0, "sauce", ""));
    host.results.borrow_mut().push(exited(0, "rice", ""));
    let output = run(&host, &fork(false, &["sauce", "rice"]), json!({}));
    assert_eq!(output, Ok(json!(["sauce", "rice"])));
    assert_eq!(
        *host.events.borrow(),
        [
            "started /do/0/cook",
            "started /do/0/cook/fork/branches/0/sauce",
            "started /do/0/cook/fork/branches/1/rice",
            "completed /do/0/cook/fork/branches/1/rice",
            "completed /do/0/cook/fork/branches/0/sauce",
            "completed /do/0/cook",
        ]
    );
}

#[test]
fn fork_branches_get_the_fork_input() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - cook:
      fork:
        branches:
          - salt:
              set:
                dish: ${ .dish + " with salt" }
          - pepper:
              set:
                dish: ${ .dish + " with pepper" }
"#,
        json!({"dish": "soup"}),
    );
    assert_eq!(
        output,
        Ok(json!([{"dish": "soup with salt"}, {"dish": "soup with pepper"}]))
    );
}

#[test]
fn a_faulting_branch_cancels_its_siblings() {
    let host = FakeHost::default();
    steps(&host, &[("simmer", None), ("burn", Some(1))]);
    host.results.borrow_mut().push(exited(3, "", "burnt\n"));
    let output = run(&host, &fork(false, &["simmer", "burn"]), json!({}));
    let Err(Halt::Fault(error)) = output else {
        panic!("expected a fault: {output:?}")
    };
    assert_eq!(
        error.instance.as_deref(),
        Some("/do/0/cook/fork/branches/1/burn")
    );
    assert_eq!(
        *host.events.borrow(),
        [
            "started /do/0/cook",
            "started /do/0/cook/fork/branches/0/simmer",
            "started /do/0/cook/fork/branches/1/burn",
            "faulted /do/0/cook/fork/branches/1/burn",
            "cancelled /do/0/cook/fork/branches/0/simmer",
            "faulted /do/0/cook",
        ]
    );
}

#[test]
fn the_first_branch_to_fault_gives_the_error() {
    let host = FakeHost::default();
    steps(&host, &[("burn", Some(2)), ("spill", Some(1))]);
    host.results.borrow_mut().push(exited(3, "", ""));
    host.results.borrow_mut().push(exited(4, "", ""));
    let output = run(&host, &fork(false, &["burn", "spill"]), json!({}));
    let Err(Halt::Fault(error)) = output else {
        panic!("expected a fault: {output:?}")
    };
    assert_eq!(
        error.instance.as_deref(),
        Some("/do/0/cook/fork/branches/1/spill")
    );
    assert!(
        host.events
            .borrow()
            .contains(&"cancelled /do/0/cook/fork/branches/0/burn".to_string())
    );
}

#[test]
fn with_compete_the_first_branch_to_complete_wins() {
    let host = FakeHost::default();
    steps(
        &host,
        &[("slow", Some(3)), ("fast", Some(1)), ("stuck", None)],
    );
    host.results.borrow_mut().push(exited(0, "slow", ""));
    host.results.borrow_mut().push(exited(0, "fast", ""));
    let output = run(&host, &fork(true, &["slow", "fast", "stuck"]), json!({}));
    assert_eq!(output, Ok(json!("fast")));
    assert_eq!(
        *host.events.borrow(),
        [
            "started /do/0/cook",
            "started /do/0/cook/fork/branches/0/slow",
            "started /do/0/cook/fork/branches/1/fast",
            "started /do/0/cook/fork/branches/2/stuck",
            "completed /do/0/cook/fork/branches/1/fast",
            "cancelled /do/0/cook/fork/branches/2/stuck",
            "cancelled /do/0/cook/fork/branches/0/slow",
            "completed /do/0/cook",
        ]
    );
}

#[test]
fn a_branch_that_completes_at_once_wins_before_the_others_start() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - pick:
      fork:
        compete: true
        branches:
          - red:
              set:
                color: red
          - green:
              set:
                color: green
"#,
        json!({}),
    );
    assert_eq!(output, Ok(json!({"color": "red"})));
    assert_eq!(
        *host.events.borrow(),
        [
            "started /do/0/pick",
            "started /do/0/pick/fork/branches/0/red",
            "completed /do/0/pick/fork/branches/0/red",
            "started /do/0/pick/fork/branches/1/green",
            "cancelled /do/0/pick/fork/branches/1/green",
            "completed /do/0/pick",
        ]
    );
}

#[test]
fn a_cancelled_branch_cancels_its_running_children() {
    let host = FakeHost::default();
    steps(&host, &[("stuck", None), ("fast", Some(1))]);
    host.results.borrow_mut().push(exited(0, "fast", ""));
    let output = run(
        &host,
        r#"
do:
  - race:
      fork:
        compete: true
        branches:
          - slow:
              do:
                - wait:
                    run:
                      shell:
                        command: stuck
                - never:
                    set: {}
          - quick:
              run:
                shell:
                  command: fast
"#,
        json!({}),
    );
    assert_eq!(output, Ok(json!("fast")));
    let events = host.events.borrow();
    for event in [
        "cancelled /do/0/race/fork/branches/0/slow/do/0/wait",
        "cancelled /do/0/race/fork/branches/0/slow",
    ] {
        assert!(events.contains(&event.to_string()), "{event}: {events:?}");
    }
    assert!(!events.iter().any(|event| event.ends_with("/never")));
}

#[test]
fn cancelling_the_run_cancels_every_branch() {
    let host = FakeHost::default();
    steps(&host, &[("simmer", None), ("stew", None)]);
    let definition: Value = serde_yaml_ng::from_str(&fork(false, &["simmer", "stew"])).unwrap();
    let mut interpreter = Interpreter::new(&host, definition, "test.run-1", "/work");
    let token = Token::new();
    let cancel = async {
        yield_now().await;
        token.cancel();
    };
    let (output, ()) = futures::executor::block_on(futures::future::join(
        interpreter.run(json!({}), &token),
        cancel,
    ));
    assert_eq!(output, Err(Halt::Cancelled));
    assert_eq!(
        *host.events.borrow(),
        [
            "started /do/0/cook",
            "started /do/0/cook/fork/branches/0/simmer",
            "started /do/0/cook/fork/branches/1/stew",
            "cancelled /do/0/cook/fork/branches/0/simmer",
            "cancelled /do/0/cook/fork/branches/1/stew",
            "cancelled /do/0/cook",
        ]
    );
}
