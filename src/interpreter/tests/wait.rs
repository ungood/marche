//! `wait` against the fake host.

use super::*;

#[test]
fn a_wait_waits_on_a_timer_and_passes_its_input_on() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - rest:
      wait: PT0.5S
  - restAgain:
      wait:
        seconds: 2
        milliseconds: 250
"#,
        json!({"dish": "soup"}),
    );
    assert_eq!(output, Ok(json!({"dish": "soup"})));
    assert_eq!(
        *host.timers.borrow(),
        [Duration::from_millis(500), Duration::from_millis(2_250)]
    );
}

#[test]
fn a_wait_can_be_an_expression() {
    let host = FakeHost::default();
    run(
        &host,
        r#"
do:
  - rest:
      wait: ${ .pause }
"#,
        json!({"pause": "PT1M"}),
    )
    .unwrap();
    assert_eq!(*host.timers.borrow(), [Duration::from_secs(60)]);
}

#[test]
fn a_duration_that_isnt_one_faults() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - rest:
      wait: ${ "soon" }
"#,
        json!({}),
    );
    let Err(Halt::Fault(error)) = output else {
        panic!("expected a fault: {output:?}")
    };
    assert_eq!(error.title.as_deref(), Some("Invalid duration"));
    assert_eq!(error.instance.as_deref(), Some("/do/0/rest"));
    assert!(host.timers.borrow().is_empty());
}

#[test]
fn cancelling_the_scope_ends_the_wait() {
    let host = FakeHost::default();
    let interpreter = Interpreter::new(&host, json!({}), "test.run-1", "/work");
    let token = Token::new();
    token.cancel();
    let output = futures::executor::block_on(interpreter.run_task(
        "rest",
        &json!({"wait": {"minutes": 10}}),
        "/do/0/rest",
        json!({}),
        &token,
        &[],
    ));
    assert_eq!(output, Err(Halt::Cancelled));
    assert_eq!(*host.timers.borrow(), [Duration::from_secs(600)]);
}
