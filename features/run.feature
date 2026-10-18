Feature: marche run

  `marche run <file>` validates a definition, starts it as a run on Temporal,
  and waits for it. It prints each task on standard error as it starts and
  finishes, and the workflow's output on standard output: a string as plain
  text, anything else as JSON. It exits with 0 when the run completes and 1
  when it faults. `--json` prints the result as one JSON object instead, and
  `--detach` prints the run ID and returns without waiting.

  Scenario: A run prints its output
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
              name: ${ .name // "world" }
        - say:
            run:
              shell:
                command: echo "Hello, $1"
                arguments:
                  - ${ .name }
      """
    When I run "marche run hello.yaml --input '{"name": "Ada"}'"
    Then the command succeeds
    And the output says:
      """
      Hello, Ada
      """

  Scenario: A run prints each task as it starts and finishes
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
    When I run "marche run hello.yaml"
    Then the command succeeds
    And marche prints:
      """
      started /do/0/greet
      completed /do/0/greet
      started /do/1/say
      completed /do/1/say
      """

  Scenario: Output that isn't a string is printed as JSON
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
              sides: 0
      """
    When I run "marche run shape.yaml"
    Then the command succeeds
    And the output says:
      """
      {
        "shape": "circle",
        "sides": 0
      }
      """

  Scenario: A faulted run fails with the task's error
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
    When I run "marche run fail.yaml"
    Then the command fails
    And the error says:
      """
      The run faulted at /do/0/check: Command failed.
      The command exited with code 3. Standard error ended with:
      no such branch
      """

  Scenario: --json prints the run's result as an object
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
    When I run "marche run shape.yaml --json"
    Then the command succeeds
    And the JSON output contains:
      """json
      {"status": "completed", "output": {"shape": "circle"}}
      """

  Scenario: --detach prints the run ID without waiting
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
                command: sleep 1
      """
    When I run "marche run nap.yaml --detach"
    Then the command succeeds
    And the output says:
      """
      examples.nap-
      """

  Scenario: A task type marche doesn't support yet is rejected before the run starts
    Given the file "ready.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: ready
        version: '0.1.0'
      do:
        - ready:
            listen:
              to:
                one:
                  with:
                    type: com.example.ready
      """
    When I run "marche run ready.yaml"
    Then the command fails
    And the error says:
      """
      ready.yaml can't run on marche yet.
      /do/0/ready is a listen task, which marche doesn't support yet.
      """

  Scenario: A definition that isn't valid OWS is rejected before the run starts
    Given the file "broken.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: broken
      do:
        - greet:
            set:
              name: world
      """
    When I run "marche run broken.yaml"
    Then the command fails
    And the error says:
      """
      broken.yaml isn't a valid workflow definition.
      /document: "version" is a required property
      """
