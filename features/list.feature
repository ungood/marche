Feature: marche list

  `marche list` lists the runs started from the current directory, newest
  first, with each run's ID, definition, status and start time. `--all` lists
  the runs started from every directory, and `-n/--limit` keeps only the
  newest. It says there are no runs only when none match. `--json` prints
  `{"runs": [{"run", "definition", "status", "started", "directory"}]}`.

  Scenario: List prints a line for each run
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
    When I run "marche list"
    Then the command succeeds
    And the output says:
      """
      $RUN examples.shape:0.1.0 completed
      """

  Scenario: Runs are listed newest first
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
        version: '0.1.0'
      do:
        - check:
            run:
              shell:
                command: exit 3
      """
    And "first.yaml" has run
    And "second.yaml" has run
    When I run "marche list --json"
    Then the command succeeds
    And the JSON output contains:
      """json
      {"runs": [
        {"run": "$RUN", "definition": "examples.second:0.1.0", "status": "faulted"},
        {"definition": "examples.first:0.1.0", "status": "completed"}
      ]}
      """

  Scenario: List leaves out runs started from other directories
    Given the file "here.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: here
        version: '0.1.0'
      do:
        - greet:
            set:
              name: here
      """
    And the file "other/elsewhere.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: elsewhere
        version: '0.1.0'
      do:
        - greet:
            set:
              name: elsewhere
      """
    And "other/elsewhere.yaml" has run
    And "here.yaml" has run
    When I run "marche list"
    Then the command succeeds
    And the output says:
      """
      examples.here:0.1.0 completed
      """
    And marche doesn't print:
      """
      examples.elsewhere
      """

  Scenario: --all lists the runs started from every directory
    Given the file "here.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: here
        version: '0.1.0'
      do:
        - greet:
            set:
              name: here
      """
    And the file "other/elsewhere.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: elsewhere
        version: '0.1.0'
      do:
        - greet:
            set:
              name: elsewhere
      """
    And "other/elsewhere.yaml" has run
    And "here.yaml" has run
    When I run "marche list --all"
    Then the command succeeds
    And the output says:
      """
      examples.here:0.1.0 completed
      examples.elsewhere:0.1.0 completed
      """

  Scenario: A directory without runs lists nothing
    When I run "marche list"
    Then the command succeeds
    And the error says:
      """
      No runs were started from this directory. `marche list --all` lists every run.
      """

  Scenario: A run that continued as new is listed once
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
    When I run "marche list --json"
    Then the command succeeds
    And the JSON output contains:
      """json
      {"runs": [{"run": "$RUN", "definition": "examples.courses:0.1.0", "status": "completed"}]}
      """

  Scenario: --limit keeps only the newest runs
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
        version: '0.1.0'
      do:
        - greet:
            set:
              name: second
      """
    And "first.yaml" has run
    And "second.yaml" has run
    When I run "marche list -n 1 --json"
    Then the command succeeds
    And the JSON output contains:
      """json
      {"runs": [{"run": "$RUN", "definition": "examples.second:0.1.0"}]}
      """

  @wip
  Scenario: A limit of 0 lists nothing, without saying there are no runs
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
    When I run "marche list --limit 0"
    Then the command succeeds
    And marche doesn't print:
      """
      $RUN
      No runs were started from this directory.
      """

  @wip
  Scenario: A namespace that doesn't exist is an error
    When I run "marche list --namespace nope"
    Then the command fails with exit code 2
    And the error says:
      """
      Temporal has no namespace nope.
      """
    And the help says:
      """
      Create it with `temporal operator namespace create nope`, or choose another with --namespace or TEMPORAL_NAMESPACE.
      """
