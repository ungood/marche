//! Continuing as new against the fake host.

use super::*;
use crate::interpreter::resume::State;

/// Runs a definition to its end, continuing as new each time the host suggests
/// it, with the state passed through JSON as Temporal carries it. Returns the
/// result and how many times the run continued as new.
fn run_chain(host: &FakeHost, definition: &str, input: Value) -> (Result<Value, Halt>, usize) {
    let definition: Value = serde_yaml_ng::from_str(definition).unwrap();
    let mut state: Option<State> = None;
    let mut continued = 0;
    loop {
        host.finished.set(0);
        let mut interpreter = Interpreter::new(host, definition.clone(), "test.run-1", "/work");
        let token = Token::new();
        let result = match state.take() {
            Some(state) => {
                futures::executor::block_on(interpreter.resume(input.clone(), state, &token))
            }
            None => futures::executor::block_on(interpreter.run(input.clone(), &token)),
        };
        if result != Err(Halt::ContinueAsNew) {
            return (result, continued);
        }
        let carried = serde_json::to_value(interpreter.continuation()).unwrap();
        state = Some(serde_json::from_value(carried).unwrap());
        continued += 1;
    }
}

const SERVICE: &str = r#"
do:
  - prepare:
      set:
        served: []
        dishes: [soup, fish, cake, bread, salad, pie]
      export:
        as: '${ { plated: 0 } }'
  - service:
      do:
        - serve:
            for:
              each: dish
              in: .dishes
            do:
              - plate:
                  set:
                    served: ${ .served + [ $dish ] }
                    startedAt: ${ $workflow.startedAt.epoch.seconds }
                  export:
                    as: '${ { plated: ($context.plated + 1) } }'
  - finish:
      set:
        served: ${ .served }
        plated: ${ $context.plated }
        table: ${ $workflow.input.table }
"#;

#[test]
fn a_loop_carries_on_in_the_next_run() {
    let host = FakeHost::default();
    host.continue_after.set(Some(3));
    let (output, continued) = run_chain(&host, SERVICE, json!({"table": 7}));
    assert_eq!(
        output,
        Ok(json!({
            "served": ["soup", "fish", "cake", "bread", "salad", "pie"],
            "plated": 6,
            "table": 7,
        }))
    );
    assert_eq!(continued, 3);
    let events = host.events.borrow();
    let plated = events
        .iter()
        .filter(|event| *event == "completed /do/1/service/do/0/serve/do/0/plate")
        .count();
    assert_eq!(plated, 6);
    // Each run starts the tasks around its position again, and finishes them
    // only once.
    let started = |reference: &str| {
        events
            .iter()
            .filter(|event| **event == format!("started {reference}"))
            .count()
    };
    assert_eq!(started("/do/0/prepare"), 1);
    assert_eq!(started("/do/1/service"), 3);
    assert_eq!(
        events
            .iter()
            .filter(|event| *event == "completed /do/1/service")
            .count(),
        1
    );
}

#[test]
fn the_carried_state_is_the_position_context_and_data() {
    let host = FakeHost::default();
    host.continue_after.set(Some(3));
    let definition: Value = serde_yaml_ng::from_str(SERVICE).unwrap();
    let mut interpreter = Interpreter::new(&host, definition, "test.run-1", "/work");
    let output = futures::executor::block_on(interpreter.run(json!({}), &Token::new()));
    assert_eq!(output, Err(Halt::ContinueAsNew));
    let state = serde_json::to_value(interpreter.continuation()).unwrap();
    assert_eq!(state["context"], json!({"plated": 2}));
    let frames: Vec<&str> = state["position"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| frame["frame"].as_str().unwrap())
        .collect();
    assert_eq!(
        frames,
        ["within", "task", "within", "task", "for", "before"]
    );
    assert_eq!(state["position"][4]["iteration"], 2);
    assert_eq!(
        state["position"][5],
        json!({"frame": "before", "index": 0, "input": {"served": ["soup", "fish"], "startedAt": 1_700_000_000}})
    );
}

#[test]
fn a_switch_loop_carries_on_in_the_next_run() {
    let host = FakeHost::default();
    host.continue_after.set(Some(1));
    for code in [1, 0, 0] {
        host.results.borrow_mut().push(exited(code, "", ""));
    }
    let (output, continued) = run_chain(
        &host,
        include_str!("../../../examples/review-loop.yaml"),
        json!({}),
    );
    assert_eq!(output, Ok(json!({"attempt": 2, "passed": true})));
    assert!(continued > 1);
}

#[test]
fn a_run_doesnt_continue_as_new_inside_a_try() {
    let host = FakeHost::default();
    host.continue_after.set(Some(1));
    let (output, continued) = run_chain(
        &host,
        r#"
do:
  - guard:
      try:
        - count:
            for:
              in: [1, 2, 3]
            do:
              - add:
                  set:
                    total: ${ .total + $item }
      catch:
        do: []
"#,
        json!({"total": 0}),
    );
    assert_eq!(output, Ok(json!({"total": 6})));
    assert_eq!(continued, 0);
}

#[test]
fn a_run_waits_for_a_parallel_loop_before_continuing_as_new() {
    let host = FakeHost::default();
    host.continue_after.set(Some(2));
    let (output, continued) = run_chain(
        &host,
        r#"
do:
  - cook:
      for:
        in: [soup, fish, cake, bread]
      metadata:
        marche:
          parallel: true
      do:
        - simmer:
            set:
              dish: ${ $item }
  - plate:
      set:
        count: ${ length }
"#,
        json!({}),
    );
    assert_eq!(output, Ok(json!({"count": 4})));
    assert_eq!(continued, 1);
    let events = host.events.borrow();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.starts_with("started /do/0/cook/do/0/simmer"))
            .count(),
        4
    );
}
