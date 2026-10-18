Feature: Catching and retrying errors

  A `try` task runs its tasks in a scope of their own. When one faults, `catch`
  decides whether to handle the error: `errors.with` lists fields the error must
  have, and `when` and `exceptWhen` are expressions that see the error as
  `$error`, or under the name `as` gives. A caught error runs the `catch.do`
  tasks, if any, and their output is the task's output.

  A `retry` policy reruns the whole `try` block after a delay, with optional
  backoff, jitter and limits. `limit.attempt.count` counts every attempt,
  including the first. When the retries run out, `catch.do` handles the last
  error, and without `catch.do` the task faults with it. Cancelling a run is
  never an error that `try` catches.

  Scenario: A retry reruns a failing step until it succeeds
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: retry-until-ready
        version: '0.1.0'
      do:
        - poll:
            try:
              - check:
                  run:
                    shell:
                      command: |
                        attempt=$(( $(cat attempts 2>/dev/null || echo 0) + 1 ))
                        echo $attempt > attempts
                        test $attempt -ge 3 && echo "ready after $attempt attempts"
            catch:
              errors:
                with:
                  type: https://open-workflow-specification.org/spec/1.0.0/errors/runtime
              retry:
                delay: PT0.1S
                backoff:
                  exponential: {}
                limit:
                  attempt:
                    count: 5
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      ready after 3 attempts
      """

  @wip
  Scenario: A step that keeps failing faults when its retries run out
    Given the file "flaky.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: retry-exhausted
        version: '0.1.0'
      do:
        - poll:
            try:
              - check:
                  run:
                    shell:
                      command: |
                        attempt=$(( $(cat attempts 2>/dev/null || echo 0) + 1 ))
                        echo $attempt > attempts
                        echo "not ready on attempt $attempt" >&2
                        exit 1
            catch:
              retry:
                delay:
                  milliseconds: 100
                limit:
                  attempt:
                    count: 2
      """
    When I run "marche run flaky.yaml"
    Then the command fails
    And the error says:
      """
      The run faulted at /do/0/poll/try/0/check: Command failed.
      The command exited with code 1. Standard error ended with:
      not ready on attempt 2
      """

  Scenario: A caught error runs the catch tasks with the error bound
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: catch-as
        version: '0.1.0'
      do:
        - attempt:
            try:
              - check:
                  run:
                    shell:
                      command: exit 3
            catch:
              errors:
                with:
                  status: 500
              as: failure
              do:
                - report:
                    set:
                      title: ${ $failure.title }
                      instance: ${ $failure.instance }
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      title: Command failed
      instance: /do/0/attempt/try/0/check
      """
    And check should fault
    And report should run after check

  Scenario: An error that doesn't match the filter isn't caught
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: catch-filter
        version: '0.1.0'
      do:
        - attempt:
            try:
              - check:
                  run:
                    shell:
                      command: exit 3
            catch:
              errors:
                with:
                  status: 404
              do:
                - report:
                    set:
                      caught: true
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/runtime
      status: 500
      instance: /do/0/attempt/try/0/check
      """

  Scenario: when and exceptWhen see the error as $error
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: catch-when
        version: '0.1.0'
      do:
        - attempt:
            try:
              - check:
                  run:
                    shell:
                      command: exit 3
            catch:
              when: ${ $error.status == 500 }
              exceptWhen: ${ $error.detail | contains("code 3") }
              do:
                - report:
                    set:
                      caught: true
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      title: Command failed
      instance: /do/0/attempt/try/0/check
      """

  @wip
  Scenario: A retry policy named from use is rejected before the run starts
    Given the file "named.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: retry-named
        version: '0.1.0'
      use:
        retries:
          patient:
            delay:
              seconds: 1
      do:
        - poll:
            try:
              - check:
                  run:
                    shell:
                      command: exit 1
            catch:
              retry: patient
      """
    When I run "marche run named.yaml"
    Then the command fails
    And the error says:
      """
      named.yaml can't run on marche yet.
      /do/0/poll uses the retry policy patient from use.retries, which marche doesn't support yet.
      """
