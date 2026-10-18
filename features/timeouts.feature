Feature: Timeouts

  A task or workflow `timeout` limits how long it runs. When the time is up,
  marche cancels everything the task started, waits until it has stopped, and
  faults with a `timeout` error. A timeout is a durable Temporal timer, so it
  survives a worker restart. `after` is written inline, as a duration object
  or an expression. Timeouts named in `use.timeouts` aren't supported yet.

  @wip
  Scenario: A task that runs past its timeout faults with a timeout error
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: task-timeout
        version: '0.1.0'
      do:
        - nap:
            run:
              shell:
                command: sleep 60
            timeout:
              after:
                seconds: 1
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/timeout
      status: 408
      title: Timed out
      detail: The task didn't finish within 1s.
      instance: /do/0/nap
      """
    And nap should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/timeout
      """

  @wip
  Scenario: A timeout cancels the tasks running inside it
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: scope-timeout
        version: '0.1.0'
      do:
        - course:
            timeout:
              after: ${ "PT1S" }
            do:
              - nap:
                  run:
                    shell:
                      command: sleep 60
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/timeout
      status: 408
      instance: /do/0/course
      """
    And course should fault
    And nap should cancel

  @wip
  Scenario: A task that finishes within its timeout completes
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: quick
        version: '0.1.0'
      do:
        - greet:
            run:
              shell:
                command: echo hello
            timeout:
              after:
                minutes: 1
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      hello
      """

  @wip
  Scenario: A workflow that runs past its timeout faults with a timeout error
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: workflow-timeout
        version: '0.1.0'
      timeout:
        after:
          seconds: 1
      do:
        - nap:
            run:
              shell:
                command: sleep 60
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/timeout
      status: 408
      title: Timed out
      detail: The workflow didn't finish within 1s.
      instance: /
      """
    And nap should cancel

  @wip
  Scenario: A named timeout is rejected before the run starts
    Given the file "nap.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: nap
        version: '0.1.0'
      do:
        - nap:
            run:
              shell:
                command: sleep 60
            timeout: short
      """
    When I run "marche run nap.yaml"
    Then the command fails
    And the error says:
      """
      nap.yaml can't run on marche yet.
      /do/0/nap uses the named timeout short, which marche doesn't support yet.
      """
