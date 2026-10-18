Feature: marche status

  `marche status <run>` shows a run's state, its definition, when it started,
  and the tasks running now. Without a run, it shows the latest run started
  from the current directory. Every command that takes a run accepts its full
  workflow ID or just its ULID. `--json` prints
  `{"run", "definition", "status", "started", "running"}`. marche asks a
  worker for the tasks of a run that's still running, so a worker must be
  serving the run's task queue.

  Scenario: Status shows the tasks running now
    Given the file "nap.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: nap
        version: '0.1.0'
      do:
        - prepare:
            set:
              seconds: 600
        - nap:
            run:
              shell:
                command: sleep 600
      """
    And "nap.yaml" is running its nap task
    When I run "marche status $RUN"
    Then the command succeeds
    And the output says:
      """
      $RUN running
      definition: examples.nap:0.1.0
      running: /do/1/nap
      """
    And marche doesn't print:
      """
      /do/0/prepare
      """

  Scenario: Without a run, status shows the latest run started from this directory
    Given the file "first.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: first
        version: '0.1.0'
      do:
        - greet:
            set:
              name: first
      """
    And the file "second.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: second
        version: '0.2.0'
      do:
        - greet:
            set:
              name: second
      """
    And "first.yaml" has run
    And "second.yaml" has run
    When I run "marche status"
    Then the command succeeds
    And the output says:
      """
      $RUN completed
      definition: examples.second:0.2.0
      """
    And marche doesn't print:
      """
      examples.first
      """

  Scenario: A run's ULID is enough
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
    When I run "marche status $ULID"
    Then the command succeeds
    And the output says:
      """
      $RUN completed
      """

  Scenario: --json prints the run's state as an object
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
    When I run "marche status $RUN --json"
    Then the command succeeds
    And the JSON output contains:
      """json
      {"run": "$RUN", "definition": "examples.shape:0.1.0", "status": "completed", "running": []}
      """

  Scenario: Without a run, status needs a run started from this directory
    When I run "marche status"
    Then the command fails with exit code 2
    And the error says:
      """
      No run was started from this directory.
      Start one with `marche run <file>`, or name a run: `marche list --all` lists every run.
      """

  Scenario: A run that doesn't exist is an error
    When I run "marche status examples.nap-01JAB3K4Z7Y8X9W0V1T2S3R4Q5"
    Then the command fails with exit code 2
    And the error says:
      """
      There's no run examples.nap-01JAB3K4Z7Y8X9W0V1T2S3R4Q5.
      `marche list` lists the runs started from this directory, and `marche list --all` lists every run.
      """

  Scenario: A ULID that no run has is an error
    When I run "marche status 01JAB3K4Z7Y8X9W0V1T2S3R4Q6"
    Then the command fails with exit code 2
    And the error says:
      """
      There's no run with the ULID 01JAB3K4Z7Y8X9W0V1T2S3R4Q6.
      `marche list` lists the runs started from this directory, and `marche list --all` lists every run.
      """

  Scenario: A ULID that more than one run has is ambiguous
    Given the run "tools.one-01JAB3K4Z7Y8X9W0V1T2S3R4Q7" has started on a task queue no worker serves
    And the run "tools.two-01JAB3K4Z7Y8X9W0V1T2S3R4Q7" has started on a task queue no worker serves
    When I run "marche status 01JAB3K4Z7Y8X9W0V1T2S3R4Q7"
    Then the command fails with exit code 2
    And the error says:
      """
      The ULID 01JAB3K4Z7Y8X9W0V1T2S3R4Q7 matches more than one run: tools.one-01JAB3K4Z7Y8X9W0V1T2S3R4Q7 and tools.two-01JAB3K4Z7Y8X9W0V1T2S3R4Q7.
      Give the run's full workflow ID.
      """

  Scenario: Status needs a worker to read the tasks of a running run
    Given the run "tools.three-01JAB3K4Z7Y8X9W0V1T2S3R4Q8" has started on a task queue no worker serves
    When I run "marche status tools.three-01JAB3K4Z7Y8X9W0V1T2S3R4Q8"
    Then the command fails with exit code 2
    And the error says:
      """
      No worker is serving the task queue elsewhere, so marche can't read the tasks of tools.three-01JAB3K4Z7Y8X9W0V1T2S3R4Q8.
      Start one with `marche worker`.
      """

  Scenario: A run that continued as new has the status of its last run
    Given the worker continues runs as new after every 1 tasks
    And the file "courses.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: courses
        version: '0.1.0'
      do:
        - soup:
            set:
              course: soup
        - fish:
            set:
              course: fish
        - cake:
            set:
              course: cake
      """
    And "courses.yaml" has run
    When I run "marche status --json"
    Then the command succeeds
    And the JSON output contains:
      """json
      {"run": "$RUN", "definition": "examples.courses:0.1.0", "status": "completed", "running": []}
      """
