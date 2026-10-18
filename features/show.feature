Feature: marche show

  `marche show <run>` lists a run's tasks in the order they started. Each
  task shows its status and reference, then its output or error, and for a
  shell step, the paths of the logs that hold its full standard output and
  error. Long output is shortened to one line; `--json` prints all of it, as
  `{"run", "status", "tasks": [{"reference", "name", "status", "output",
  "error", "log"}]}`. marche asks a worker for the tasks, even after the run
  has ended.

  Scenario: Show lists each task with its output and logs
    Given the file "hello.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: hello
        version: '0.1.0'
      do:
        - greet:
            set:
              name: world
        - say:
            run:
              shell:
                command: echo "Hello, $1"
                arguments:
                  - ${ .name }
      """
    And "hello.yaml" has run
    When I run "marche show $RUN"
    Then the command succeeds
    And the output says:
      """
      $RUN completed
      completed /do/0/greet
      output: {"name":"world"}
      completed /do/1/say
      output: "Hello, world"
      stdout:
      /runs/$RUN/do/1/say/1.stdout
      stderr:
      /runs/$RUN/do/1/say/1.stderr
      """

  Scenario: Show prints a faulted task's error
    Given the file "fail.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: fail
        version: '0.1.0'
      do:
        - check:
            run:
              shell:
                command: echo "no such branch" >&2; exit 3
      """
    And "fail.yaml" has run
    When I run "marche show $RUN"
    Then the command succeeds
    And the output says:
      """
      $RUN faulted
      faulted /do/0/check
      error: Command failed
      The command exited with code 3. Standard error ended with:
      no such branch
      """

  Scenario: --json prints every task as an object
    Given the file "hello.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: hello
        version: '0.1.0'
      do:
        - greet:
            set:
              name: world
        - say:
            run:
              shell:
                command: echo "Hello, $1"
                arguments:
                  - ${ .name }
      """
    And "hello.yaml" has run
    When I run "marche show $RUN --json"
    Then the command succeeds
    And the JSON output contains:
      """json
      {"run": "$RUN", "status": "completed", "tasks": [
        {"reference": "/do/0/greet", "name": "greet", "status": "completed", "output": {"name": "world"}, "error": null, "log": null},
        {"reference": "/do/1/say", "name": "say", "status": "completed", "output": "Hello, world", "error": null}
      ]}
      """

  Scenario: Show accepts a run's ULID
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
    When I run "marche show $ULID"
    Then the command succeeds
    And the output says:
      """
      $RUN completed
      completed /do/0/setShape
      """
