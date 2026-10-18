Feature: marche cancel

  `marche cancel <run>` asks Temporal to cancel a run and waits until it has
  stopped. Cancelling leaves the run's outermost scope, so every task in it is
  cancelled, and a shell step's whole process group is sent SIGTERM, then
  SIGKILL ten seconds later. When `marche cancel` returns, every process the
  run started has exited. Cancelling a run that has already ended changes
  nothing. `--json` prints `{"run", "status"}`.

  Only a worker can stop a run, so when no worker is serving the run's task
  queue, `marche cancel` requests the cancellation and returns at once, and the
  run stops once a worker picks it up. Its status is then still `running`. A
  worker's processes die with it, so none of the run's processes are left
  running meanwhile. A run that was terminated, as `temporal workflow
  terminate` does, counts as cancelled.

  Scenario: Cancel stops every process the run started
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
                command: sleep 600 & echo $! > nap.pid; wait
      """
    And "nap.yaml" is running its nap task
    And the workflow has written the file "nap.pid"
    When I run "marche cancel $RUN"
    Then the command succeeds
    And the output says:
      """
      $RUN was cancelled.
      """
    And the process whose ID is in "nap.pid" has stopped

  Scenario: --json prints the run's final status
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
                command: sleep 600
      """
    And "nap.yaml" is running its nap task
    When I run "marche cancel $ULID --json"
    Then the command succeeds
    And the JSON output contains:
      """json
      {"run": "$RUN", "status": "cancelled"}
      """

  Scenario: Cancelling a run that has ended changes nothing
    Given the file "shape.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: shape
        version: '0.1.0'
      do:
        - setShape:
            set:
              shape: circle
      """
    And "shape.yaml" has run
    When I run "marche cancel $RUN"
    Then the command succeeds
    And the output says:
      """
      $RUN had already completed.
      """

  Scenario: Cancelling a run that doesn't exist is an error
    When I run "marche cancel examples.nap-01JAB3K4Z7Y8X9W0V1T2S3R4Q9"
    Then the command fails with exit code 2
    And the error says:
      """
      There's no run examples.nap-01JAB3K4Z7Y8X9W0V1T2S3R4Q9.
      """

  Scenario: Cancelling a run that has been terminated changes nothing
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
                command: sleep 600
      """
    And "nap.yaml" is running its nap task
    And the run has been terminated
    When I run "marche cancel $RUN"
    Then the command succeeds
    And the output says:
      """
      $RUN had already been cancelled.
      """

  Scenario: Without a worker, cancel returns once the cancellation is requested
    Given the run "tools.four-01JAB3K4Z7Y8X9W0V1T2S3R4QA" has started on a task queue no worker serves
    When I run "marche cancel tools.four-01JAB3K4Z7Y8X9W0V1T2S3R4QA"
    Then the command succeeds
    And the output says:
      """
      Cancellation of tools.four-01JAB3K4Z7Y8X9W0V1T2S3R4QA is requested. No worker is serving the task queue elsewhere, so the run will stop once one picks it up.
      """

  Scenario: Without a worker, --json says the run is still running
    Given the run "tools.five-01JAB3K4Z7Y8X9W0V1T2S3R4QB" has started on a task queue no worker serves
    When I run "marche cancel tools.five-01JAB3K4Z7Y8X9W0V1T2S3R4QB --json"
    Then the command succeeds
    And the JSON output contains:
      """json
      {"run": "tools.five-01JAB3K4Z7Y8X9W0V1T2S3R4QB", "status": "running"}
      """
