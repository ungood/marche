//! `for` against the fake host.

use super::*;

#[test]
fn iterations_chain_their_output_with_the_item_and_index_bound() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - loopColors:
      for:
        each: color
        in: .colors
      do:
        - markProcessed:
            set:
              processed: '${ { colors: (.processed.colors + [ $color ]), indexes: (.processed.indexes + [ $index ]) } }'
"#,
        json!({"colors": ["red", "green", "blue"]}),
    );
    assert_eq!(
        output,
        Ok(json!({"processed": {"colors": ["red", "green", "blue"], "indexes": [0, 1, 2]}}))
    );
    assert_eq!(
        *host.events.borrow(),
        [
            "started /do/0/loopColors",
            "started /do/0/loopColors/do/0/markProcessed",
            "completed /do/0/loopColors/do/0/markProcessed",
            "started /do/0/loopColors/do/0/markProcessed",
            "completed /do/0/loopColors/do/0/markProcessed",
            "started /do/0/loopColors/do/0/markProcessed",
            "completed /do/0/loopColors/do/0/markProcessed",
            "completed /do/0/loopColors",
        ]
    );
}

#[test]
fn while_is_checked_before_each_iteration() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - fill:
      for:
        each: amount
        at: position
        in: [4, 3, 5, 2]
      while: ${ .total + $amount <= 10 }
      do:
        - add:
            set:
              total: ${ .total + $amount }
              last: ${ $position }
"#,
        json!({"total": 0}),
    );
    assert_eq!(output, Ok(json!({"total": 7, "last": 1})));
}

#[test]
fn an_empty_collection_passes_the_input_on() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - serve:
      for:
        in: .dishes
      do:
        - plate:
            set:
              plated: true
"#,
        json!({"dishes": []}),
    );
    assert_eq!(output, Ok(json!({"dishes": []})));
}

#[test]
fn a_collection_that_isnt_an_array_faults() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - serve:
      for:
        in: .dishes
      do: []
"#,
        json!({"dishes": {"soup": 1}}),
    );
    let Err(Halt::Fault(error)) = output else {
        panic!("expected a fault: {output:?}")
    };
    assert_eq!(error.status, 400);
    assert_eq!(error.instance.as_deref(), Some("/do/0/serve"));
    assert_eq!(
        error.detail.as_deref(),
        Some(r#"for.in must give an array, but it gave {"soup":1}."#)
    );
}

#[test]
fn end_inside_an_iteration_ends_the_workflow() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - serve:
      for:
        in: [1, 2, 3]
      do:
        - stop:
            set:
              stopped: ${ $item }
            then: end
  - never:
      set:
        never: true
"#,
        json!({}),
    );
    assert_eq!(output, Ok(json!({"stopped": 1})));
}

#[test]
fn a_failing_iteration_stops_the_loop() {
    let host = FakeHost::default();
    host.results.borrow_mut().push(exited(0, "", ""));
    host.results.borrow_mut().push(exited(1, "", "burned"));
    let output = run(
        &host,
        r#"
do:
  - serve:
      for:
        in: [soup, fish, cake]
      do:
        - cook:
            run:
              shell:
                command: ./cook
"#,
        json!({}),
    );
    let Err(Halt::Fault(error)) = output else {
        panic!("expected a fault: {output:?}")
    };
    assert_eq!(error.instance.as_deref(), Some("/do/0/serve/do/0/cook"));
    assert_eq!(host.requests.borrow().len(), 2);
}

/// A parallel loop over the dishes, whose iterations run `./cook <dish>`, with
/// one more setting under `metadata.marche`.
fn cook(setting: &str) -> String {
    format!(
        r#"
do:
  - cook:
      for:
        each: dish
        in: .dishes
      metadata:
        marche:
          parallel: true
          {setting}
      do:
        - simmer:
            run:
              shell:
                command: ${{ "./cook " + $dish }}
"#
    )
}

fn simmer(status: &str) -> String {
    format!("{status} /do/0/cook/do/0/simmer")
}

#[test]
fn parallel_output_keeps_item_order() {
    let host = FakeHost::default();
    steps(
        &host,
        &[
            ("./cook soup", Some(3)),
            ("./cook fish", Some(1)),
            ("./cook cake", Some(0)),
        ],
    );
    for dish in ["soup", "fish", "cake"] {
        host.results.borrow_mut().push(exited(0, dish, ""));
    }
    let output = run(
        &host,
        &cook(""),
        json!({"dishes": ["soup", "fish", "cake"]}),
    );
    assert_eq!(output, Ok(json!(["soup", "fish", "cake"])));
    assert_eq!(
        *host.events.borrow(),
        [
            "started /do/0/cook".to_string(),
            simmer("started"),
            simmer("started"),
            simmer("started"),
            simmer("completed"),
            simmer("completed"),
            simmer("completed"),
            "completed /do/0/cook".to_string(),
        ]
    );
}

#[test]
fn parallel_iterations_get_the_task_input_with_the_item_bound() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - cook:
      for:
        in: .dishes
      metadata:
        marche:
          parallel: true
      do:
        - plate:
            set:
              dish: ${ $item }
              at: ${ $index }
              table: ${ .table }
"#,
        json!({"table": 4, "dishes": ["soup", "fish"]}),
    );
    assert_eq!(
        output,
        Ok(json!([
            {"dish": "soup", "at": 0, "table": 4},
            {"dish": "fish", "at": 1, "table": 4},
        ]))
    );
}

#[test]
fn max_concurrency_starts_an_iteration_when_a_slot_frees() {
    let host = FakeHost::default();
    steps(
        &host,
        &[
            ("./cook soup", Some(3)),
            ("./cook fish", Some(1)),
            ("./cook cake", Some(1)),
        ],
    );
    for _ in 0..3 {
        host.results.borrow_mut().push(exited(0, "", ""));
    }
    run(
        &host,
        &cook("maxConcurrency: 2"),
        json!({"dishes": ["soup", "fish", "cake"]}),
    )
    .unwrap();
    let commands: Vec<String> = host
        .requests
        .borrow()
        .iter()
        .map(|request| request.command.clone())
        .collect();
    assert_eq!(commands, ["./cook soup", "./cook fish", "./cook cake"]);
    // The third iteration starts only once the second has finished.
    assert_eq!(
        host.events.borrow()[1..5],
        [
            simmer("started"),
            simmer("started"),
            simmer("completed"),
            simmer("started"),
        ]
    );
}

#[test]
fn the_first_iteration_to_fault_cancels_the_others() {
    let host = FakeHost::default();
    steps(
        &host,
        &[
            ("./cook soup", None),
            ("./cook burnt", Some(1)),
            ("./cook cake", Some(0)),
        ],
    );
    host.results.borrow_mut().push(exited(1, "", "burned"));
    let output = run(
        &host,
        &cook("maxConcurrency: 2"),
        json!({"dishes": ["soup", "burnt", "cake"]}),
    );
    let Err(Halt::Fault(error)) = output else {
        panic!("expected a fault: {output:?}")
    };
    assert!(error.detail.unwrap().ends_with("burned"));
    // No more iterations start after the fault.
    assert_eq!(host.requests.borrow().len(), 2);
    assert_eq!(
        *host.events.borrow(),
        [
            "started /do/0/cook".to_string(),
            simmer("started"),
            simmer("started"),
            simmer("faulted"),
            simmer("cancelled"),
            "faulted /do/0/cook".to_string(),
        ]
    );
}

#[test]
fn without_cancel_on_failure_the_lowest_indexed_fault_wins() {
    let host = FakeHost::default();
    steps(
        &host,
        &[
            ("./cook soup", Some(3)),
            ("./cook fish", Some(0)),
            ("./cook cake", Some(1)),
        ],
    );
    host.results.borrow_mut().push(exited(1, "", "soup burned"));
    host.results.borrow_mut().push(exited(1, "", "fish burned"));
    host.results.borrow_mut().push(exited(0, "cake", ""));
    let output = run(
        &host,
        &cook("cancelOnFailure: false"),
        json!({"dishes": ["soup", "fish", "cake"]}),
    );
    let Err(Halt::Fault(error)) = output else {
        panic!("expected a fault: {output:?}")
    };
    assert!(error.detail.unwrap().ends_with("soup burned"));
    let events = host.events.borrow();
    assert!(events.contains(&simmer("completed")));
    assert!(!events.contains(&simmer("cancelled")));
}

#[test]
fn cancelling_the_run_cancels_running_iterations() {
    let host = FakeHost::default();
    steps(&host, &[("./cook soup", None), ("./cook fish", None)]);
    let definition: Value = serde_yaml_ng::from_str(&cook("")).unwrap();
    let mut interpreter = Interpreter::new(&host, definition, "test.run-1", "/work");
    let token = Token::new();
    let run = interpreter.run(json!({"dishes": ["soup", "fish"]}), &token);
    let cancel = async {
        yield_now().await;
        token.cancel();
    };
    let (output, ()) = futures::executor::block_on(async { futures::join!(run, cancel) });
    assert_eq!(output, Err(Halt::Cancelled));
    assert_eq!(
        *host.events.borrow(),
        [
            "started /do/0/cook".to_string(),
            simmer("started"),
            simmer("started"),
            simmer("cancelled"),
            simmer("cancelled"),
            "cancelled /do/0/cook".to_string(),
        ]
    );
}
