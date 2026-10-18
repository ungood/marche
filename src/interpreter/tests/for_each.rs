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
