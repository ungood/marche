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
    And the output says, in order:
      """
      $RUN completed
      completed /do/0/greet
      output: {"name":"world"}
      completed /do/1/say
      output: "Hello, world"
      stdout:
      /runs/$RUN/do/1/say/1/1.stdout
      stderr:
      /runs/$RUN/do/1/say/1/1.stderr
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
    And the output says, in order:
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

  Scenario: Long output is shortened to one line
    Given the file "count.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: count
        version: '0.1.0'
      do:
        - count:
            run:
              shell:
                command: echo $(seq 1 100)
      """
    And "count.yaml" has run
    When I run "marche show $RUN"
    Then the command succeeds
    And the output says:
      """
      output: "1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63 64 65 66 67 68 69 7...
      """
    And marche doesn't print:
      """
      69 70
      """

  Scenario: -v shows all of a task's output
    Given the file "count.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: count
        version: '0.1.0'
      do:
        - count:
            run:
              shell:
                command: echo $(seq 1 100)
      """
    And "count.yaml" has run
    When I run "marche show $RUN -v"
    Then the command succeeds
    And the output says:
      """
      output: "1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63 64 65 66 67 68 69 70 71 72 73 74 75 76 77 78 79 80 81 82 83 84 85 86 87 88 89 90 91 92 93 94 95 96 97 98 99 100"
      """
