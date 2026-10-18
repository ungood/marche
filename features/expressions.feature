Feature: Expressions and data flow

  Expressions are jq, evaluated by jaq. In values such as `set` and shell
  arguments, only strings written as `${ }` are expressions. Fields that are
  always expressions, such as `if`, `input.from`, `output.as` and `export.as`,
  may leave out the `${ }`. Expressions see the spec's arguments: `$context`,
  `$input`, `$output`, `$task`, `$workflow` and `$runtime`.

  A run must replay the same way every time, so expressions can't read the
  clock, the environment or the machine's time zone, and can't write to the
  worker's output: `now`, `env`, `$ENV`, `input`, `inputs`, `debug`,
  `stderr`, `halt`, `halt_error`, `localtime` and `strflocaltime` aren't
  available. Times come from `$workflow.startedAt` and `$task.startedAt`,
  which Temporal records.

  Scenario: if skips a task whose condition is false
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: if
        version: '0.1.0'
      do:
        - start:
            set:
              hungry: false
        - eat:
            if: .hungry
            set:
              ate: true
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      hungry: false
      """
    And start should run first
    And start should run last

  Scenario: then exit leaves the enclosing do
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: exit
        version: '0.1.0'
      do:
        - course:
            do:
              - starter:
                  set:
                    served: [ starter ]
                  then: exit
              - skipped:
                  set:
                    served: [ never ]
        - dessert:
            set:
              served: ${ .served + [ "dessert" ] }
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      served: [ starter, dessert ]
      """

  Scenario: then end finishes the workflow
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: end
        version: '0.1.0'
      do:
        - course:
            do:
              - starter:
                  set:
                    served: [ starter ]
                  then: end
        - dessert:
            set:
              served: [ dessert ]
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      served: [ starter ]
      """

  Scenario: export.as writes the context that later tasks read
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: context
        version: '0.1.0'
      do:
        - order:
            set:
              dish: soup
            export:
              as: '$context + { order: .dish }'
        - serve:
            set:
              served: ${ $context.order }
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      served: soup
      """

  Scenario: Workflow input.from and output.as shape the run's input and output
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: workflow-data
        version: '0.1.0'
      input:
        from: .order
      output:
        as: .dish
      do:
        - cook:
            set:
              dish: ${ "hot " + .dish }
      """
    And given the workflow input is:
      """yaml
      order:
        dish: soup
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      hot soup
      """

  Scenario: Expressions see the task, the workflow and the runtime
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: arguments
        version: '0.1.0'
      do:
        - describe:
            set:
              task: ${ $task.name }
              reference: ${ $task.reference }
              workflow: ${ $workflow.definition.document.name }
              runtime: ${ $runtime.name }
              timed: ${ $task.startedAt.epoch.milliseconds >= $workflow.startedAt.epoch.milliseconds }
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      task: describe
      reference: /do/0/describe
      workflow: arguments
      runtime: marche
      timed: true
      """

  Scenario: An expression can't read the clock
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: clock
        version: '0.1.0'
      do:
        - stamp:
            set:
              at: ${ now }
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/expression
      status: 400
      title: Expression failed
      detail: '`now` isn''t available in marche expressions, because a run must replay the same way every time. Use $workflow.startedAt or $task.startedAt instead.'
      instance: /do/0/stamp
      """

  Scenario: A failing expression faults the task with an expression error
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: bad-expression
        version: '0.1.0'
      do:
        - divide:
            set:
              half: ${ .missing | error("no half") }
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/expression
      status: 400
      title: Expression failed
      instance: /do/0/divide
      """
