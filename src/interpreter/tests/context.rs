//! `$context` as tasks see it after the tasks inside them have exported.

use super::*;

#[test]
fn output_as_sees_the_context_inner_tasks_exported() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - outer:
      do:
        - inner:
            set: {}
            export:
              as: '{ a: 1 }'
      output:
        as: '$context'
"#,
        json!({}),
    );
    assert_eq!(output, Ok(json!({"a": 1})));
}

#[test]
fn export_without_as_keeps_the_context_inner_tasks_exported() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - outer:
      do:
        - inner:
            set: {}
            export:
              as: '{ a: 1 }'
      export:
        schema:
          format: json
          document:
            type: object
            required: [ a ]
  - read:
      set:
        context: ${ $context }
"#,
        json!({}),
    );
    assert_eq!(output, Ok(json!({"context": {"a": 1}})));
}

#[test]
fn export_as_sees_the_context_inner_tasks_exported() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - outer:
      do:
        - inner:
            set: {}
            export:
              as: '{ a: 1 }'
      export:
        as: '$context + { b: 2 }'
  - read:
      set:
        context: ${ $context }
"#,
        json!({}),
    );
    assert_eq!(output, Ok(json!({"context": {"a": 1, "b": 2}})));
}

#[test]
fn while_sees_the_context_the_last_iteration_exported() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - count:
      for:
        each: x
        in: [ 1, 2, 3, 4, 5 ]
      while: '$context.stop != true'
      do:
        - step:
            set:
              n: ${ $x }
            export:
              as: '{ stop: ($x >= 2) }'
"#,
        json!({}),
    );
    assert_eq!(output, Ok(json!({"n": 2})));
}

#[test]
fn catch_when_sees_the_context_the_try_exported() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - guarded:
      try:
        - mark:
            set: {}
            export:
              as: '{ handled: true }'
        - fail:
            raise:
              error:
                type: https://example.com/oops
                status: 500
      catch:
        when: '$context.handled'
        do:
          - recover:
              set:
                recovered: true
"#,
        json!({}),
    );
    assert_eq!(output, Ok(json!({"recovered": true})));
}

#[test]
fn retry_when_sees_the_context_the_try_exported() {
    let host = FakeHost::default();
    let output = run(
        &host,
        r#"
do:
  - guarded:
      try:
        - mark:
            set: {}
            export:
              as: '{ tries: (($context.tries // 0) + 1) }'
        - fail:
            if: '$context.tries < 3'
            raise:
              error:
                type: https://example.com/oops
                status: 500
        - done:
            set:
              tries: ${ $context.tries }
      catch:
        retry:
          exceptWhen: '$context.tries >= 2'
          delay:
            seconds: 1
"#,
        json!({}),
    );
    let Err(Halt::Fault(error)) = output else {
        panic!("expected a fault: {output:?}")
    };
    assert_eq!(error.kind, "https://example.com/oops");
    assert_eq!(*host.timers.borrow(), [Duration::from_secs(1)]);
}

#[test]
fn a_branch_export_sees_what_an_earlier_branch_exported() {
    let host = FakeHost::default();
    steps(&host, &[("slow", Some(2))]);
    host.results.borrow_mut().push(exited(0, "slow", ""));
    let output = run(
        &host,
        r#"
do:
  - cook:
      fork:
        branches:
          - slow:
              run:
                shell:
                  command: slow
              export:
                as: '$context + { slow: true }'
          - quick:
              set: {}
              export:
                as: '$context + { quick: true }'
  - read:
      set:
        context: ${ $context }
"#,
        json!({}),
    );
    assert_eq!(
        output,
        Ok(json!({"context": {"quick": true, "slow": true}}))
    );
}
