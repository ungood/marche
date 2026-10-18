//! `try`, `catch` and `retry` against the fake host.

use super::*;

/// A `try` around one shell step, with the given `catch`, indented to fit.
fn guarded(catch: &str) -> String {
    let catch: String = catch
        .lines()
        .map(|line| format!("        {line}\n"))
        .collect();
    format!(
        r#"
do:
  - poll:
      try:
        - check:
            run:
              shell:
                command: ./check
      catch:
{catch}"#
    )
}

/// Scripts the shell: `failures` failed attempts, then one that succeeds.
fn attempts(host: &FakeHost, failures: usize) {
    let mut results = host.results.borrow_mut();
    for attempt in 1..=failures {
        results.push(exited(1, "", &format!("not ready on attempt {attempt}")));
    }
    results.push(exited(0, "ready", ""));
}

fn seconds(seconds: &[u64]) -> Vec<Duration> {
    seconds.iter().copied().map(Duration::from_secs).collect()
}

fn fault(output: Result<Value, Halt>) -> ows::Error {
    match output {
        Err(Halt::Fault(error)) => error,
        other => panic!("expected a fault: {other:?}"),
    }
}

#[test]
fn a_caught_error_runs_the_catch_tasks_with_the_error_bound() {
    let host = FakeHost::default();
    attempts(&host, 1);
    let output = run(
        &host,
        &guarded(
            r#"
errors:
  with:
    type: https://open-workflow-specification.org/spec/1.0.0/errors/runtime
    status: 500
as: failure
do:
  - report:
      set:
        instance: ${ $failure.instance }
        dish: ${ .dish }
  - nested:
      do:
        - still:
            set:
              title: ${ $failure.title }
"#,
        ),
        json!({"dish": "soup"}),
    );
    assert_eq!(output, Ok(json!({"title": "Command failed"})));
    assert_eq!(
        *host.events.borrow(),
        [
            "started /do/0/poll",
            "started /do/0/poll/try/0/check",
            "faulted /do/0/poll/try/0/check",
            "started /do/0/poll/catch/do/0/report",
            "completed /do/0/poll/catch/do/0/report",
            "started /do/0/poll/catch/do/1/nested",
            "started /do/0/poll/catch/do/1/nested/do/0/still",
            "completed /do/0/poll/catch/do/1/nested/do/0/still",
            "completed /do/0/poll/catch/do/1/nested",
            "completed /do/0/poll"
        ]
    );
}

#[test]
fn the_error_is_bound_as_error_by_default() {
    let host = FakeHost::default();
    attempts(&host, 1);
    let output = run(
        &host,
        &guarded(
            r#"
do:
  - report:
      set: ${ $error.status }
"#,
        ),
        json!({}),
    );
    assert_eq!(output, Ok(json!(500)));
}

#[test]
fn a_caught_error_without_catch_tasks_passes_the_input_on() {
    let host = FakeHost::default();
    attempts(&host, 1);
    let output = run(&host, &guarded("errors: {}"), json!({"dish": "soup"}));
    assert_eq!(output, Ok(json!({"dish": "soup"})));
}

#[test]
fn errors_that_dont_match_are_not_caught() {
    for catch in [
        "errors:\n  with:\n    status: 404",
        "when: ${ $error.status == 404 }",
        "exceptWhen: ${ $error.status == 500 }",
    ] {
        let host = FakeHost::default();
        attempts(&host, 1);
        let error = fault(run(
            &host,
            &guarded(&format!("{catch}\ndo:\n  - report:\n      set: {{}}")),
            json!({}),
        ));
        assert_eq!(
            error.instance.as_deref(),
            Some("/do/0/poll/try/0/check"),
            "{catch}"
        );
        assert!(
            !host.events.borrow().iter().any(|e| e.contains("report")),
            "{catch}"
        );
    }
}

#[test]
fn a_retry_reruns_the_block_until_it_succeeds() {
    let host = FakeHost::default();
    attempts(&host, 2);
    let output = run(
        &host,
        &guarded(
            r#"
retry:
  delay: PT1S
"#,
        ),
        json!({}),
    );
    assert_eq!(output, Ok(json!("ready")));
    assert_eq!(host.requests.borrow().len(), 3);
    assert_eq!(*host.timers.borrow(), seconds(&[1, 1]));
}

#[test]
fn backoff_grows_the_delay() {
    for (backoff, expected) in [
        ("constant", [2, 2, 2, 2]),
        ("linear", [2, 4, 6, 8]),
        ("exponential", [2, 4, 8, 16]),
    ] {
        let host = FakeHost::default();
        attempts(&host, 4);
        run(
            &host,
            &guarded(&format!(
                "retry:\n  delay:\n    seconds: 2\n  backoff:\n    {backoff}: {{}}"
            )),
            json!({}),
        )
        .unwrap();
        assert_eq!(*host.timers.borrow(), seconds(&expected), "{backoff}");
    }
}

#[test]
fn jitter_adds_a_random_part_of_its_range() {
    let host = FakeHost::default();
    attempts(&host, 1);
    run(
        &host,
        &guarded(
            r#"
retry:
  delay: PT1S
  jitter:
    from: PT1S
    to: PT3S
"#,
        ),
        json!({}),
    )
    .unwrap();
    // The fake host's random numbers are always 0.5.
    assert_eq!(*host.timers.borrow(), seconds(&[3]));
    assert_eq!(*host.streams.borrow(), ["marche.jitter"]);
}

#[test]
fn the_attempt_limit_counts_every_attempt() {
    let host = FakeHost::default();
    attempts(&host, 5);
    let error = fault(run(
        &host,
        &guarded(
            r#"
retry:
  delay: PT1S
  limit:
    attempt:
      count: 3
"#,
        ),
        json!({}),
    ));
    assert_eq!(host.requests.borrow().len(), 3);
    assert_eq!(*host.timers.borrow(), seconds(&[1, 1]));
    assert!(error.detail.unwrap().ends_with("not ready on attempt 3"));
}

#[test]
fn the_duration_limit_stops_retries_that_would_end_after_it() {
    let host = FakeHost::default();
    attempts(&host, 5);
    fault(run(
        &host,
        &guarded(
            r#"
retry:
  delay:
    seconds: 10
  limit:
    duration: PT25S
"#,
        ),
        json!({}),
    ));
    assert_eq!(host.requests.borrow().len(), 3);
    assert_eq!(*host.timers.borrow(), seconds(&[10, 10]));
}

#[test]
fn retry_when_decides_whether_to_retry() {
    let host = FakeHost::default();
    attempts(&host, 5);
    fault(run(
        &host,
        &guarded(
            r#"
retry:
  delay: PT1S
  when: ${ $error.detail | endswith("attempt 1") }
"#,
        ),
        json!({}),
    ));
    assert_eq!(host.requests.borrow().len(), 2);
}

#[test]
fn catch_tasks_handle_the_error_once_retries_run_out() {
    let host = FakeHost::default();
    attempts(&host, 5);
    let output = run(
        &host,
        &guarded(
            r#"
as: failure
retry:
  delay: PT1S
  limit:
    attempt:
      count: 2
do:
  - giveUp:
      set: ${ $failure.detail }
"#,
        ),
        json!({}),
    );
    assert_eq!(
        output,
        Ok(json!(
            "The command exited with code 1. Standard error ended with:\nnot ready on attempt 2"
        ))
    );
}

#[test]
fn catch_then_picks_the_flow_after_a_caught_error() {
    let host = FakeHost::default();
    attempts(&host, 1);
    let mut definition = guarded("then: end");
    definition.push_str("  - skipped:\n      set:\n        ran: true\n");
    let output = run(&host, &definition, json!({"dish": "soup"}));
    assert_eq!(output, Ok(json!({"dish": "soup"})));
}

#[test]
fn cancellation_is_never_caught() {
    let host = FakeHost::default();
    let definition: Value = serde_yaml_ng::from_str(&guarded(
        r#"
do:
  - report:
      set: {}
"#,
    ))
    .unwrap();
    let interpreter = Interpreter::new(&host, definition.clone(), "test.run-1", "/work");
    let token = Token::new();
    token.cancel();
    let output = futures::executor::block_on(interpreter.run_task(
        "poll",
        &definition["do"][0]["poll"],
        "/do/0/poll",
        json!({}),
        &token,
        &[],
    ));
    assert_eq!(output, Err(Halt::Cancelled));
    assert_eq!(
        *host.events.borrow(),
        ["started /do/0/poll", "faulted /do/0/poll"]
    );
}
