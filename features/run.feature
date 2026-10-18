Feature: marche run

  `marche run <file>` makes the checks `marche validate` makes, starts the
  definition as a run on Temporal, and waits for it. It prints each task on
  standard error as it starts and finishes, and the workflow's output on
  standard output: a string as plain text, anything else as JSON. `-v` adds
  each finished task's output or error to its line, and `-q` prints only the
  workflow's output. `--json` prints the result as one JSON object instead, and
  `--detach` prints the run ID and returns without waiting.

  It exits with 0 when the run completes, 1 when it faults or is cancelled or
  the definition can't run, and 2 when marche can't do what it was asked, such
  as when `--input` isn't JSON or Temporal can't be reached.

  marche finds Temporal the way Temporal's CLI does: the profile in
  `temporal.toml` named by `--profile` or `TEMPORAL_PROFILE`, then
  `TEMPORAL_ADDRESS` and `TEMPORAL_NAMESPACE`, then `--address` and
  `--namespace`, each overriding the ones before.

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
    And marche prints, in order:
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
    Then the command fails with exit code 1
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
    Given the file "fetch.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: fetch
        version: '0.1.0'
      do:
        - fetch:
            call: http
            with:
              method: get
              endpoint: https://example.com
      """
    When I run "marche run fetch.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      fetch.yaml can't run on marche yet.
      /do/0/fetch is a call task, which marche doesn't support yet.
      """
    And the error points at line 8 of "fetch.yaml"

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
    Then the command fails with exit code 1
    And the error says:
      """
      broken.yaml isn't a valid workflow definition.
      /document: "version" is a required property
      """

  Scenario: -v adds each task's output to its line
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
    When I run "marche run hello.yaml -v"
    Then the command succeeds
    And marche prints:
      """
      started /do/0/greet
      completed /do/0/greet: {"name":"world"}
      completed /do/1/say: "Hello, world"
      """

  Scenario: -v adds a faulted task's error to its line
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
    When I run "marche run fail.yaml -v"
    Then the command fails with exit code 1
    And the error says:
      """
      faulted /do/0/check: Command failed: The command exited with code 3.
      """

  Scenario: -q prints only the workflow's output
    Given the file "hello.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: hello
        version: '0.1.0'
      do:
        - say:
            run:
              shell:
                command: echo "Hello, world"
      """
    When I run "marche run hello.yaml -q"
    Then the command succeeds
    And the output says:
      """
      Hello, world
      """
    And marche doesn't print:
      """
      started /do/0/say
      """

  Scenario: --detach with --json prints the run as an object
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
    When I run "marche run nap.yaml --detach --json"
    Then the command succeeds
    And the JSON output contains:
      """json
      {"status": "running"}
      """

  Scenario: --input that isn't JSON is rejected before the run starts
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
              name: ${ .name }
      """
    When I run "marche run hello.yaml --input Ada"
    Then the command fails with exit code 2
    And the error says:
      """
      --input isn't JSON: expected value at line 1 column 1.
      """
    And the help says:
      """
      Quote the JSON for your shell, as in --input '{"name": "Ada"}'.
      """

  Scenario: --address overrides TEMPORAL_ADDRESS
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
      """
    When I run "marche run hello.yaml --address 127.0.0.1:1"
    Then the command fails with exit code 2
    And the error says:
      """
      Couldn't connect to Temporal at 127.0.0.1:1
      """
    And the help says:
      """
      Start a server with `temporal server start-dev`, or point marche at one with --address or TEMPORAL_ADDRESS.
      """

  Scenario: TEMPORAL_NAMESPACE picks the namespace
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
      """
    And the environment variable "TEMPORAL_NAMESPACE" is "kitchen"
    When I run "marche run hello.yaml"
    Then the command fails with exit code 2
    And the error says:
      """
      Temporal has no namespace kitchen.
      """
    And the help says:
      """
      Create it with `temporal operator namespace create kitchen`, or choose another with --namespace or TEMPORAL_NAMESPACE.
      """

  Scenario: --namespace overrides TEMPORAL_NAMESPACE
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
      """
    And the environment variable "TEMPORAL_NAMESPACE" is "kitchen"
    When I run "marche run hello.yaml --namespace default --json"
    Then the command succeeds
    And the JSON output contains:
      """json
      {"status": "completed", "output": {"name": "world"}}
      """

  Scenario: The default profile in Temporal's configuration file applies
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
      """
    And Temporal's configuration file says:
      """toml
      [profile.default]
      namespace = "kitchen"
      """
    When I run "marche run hello.yaml"
    Then the command fails with exit code 2
    And the error says:
      """
      Temporal has no namespace kitchen.
      """

  Scenario: --profile picks a profile in Temporal's configuration file
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
      """
    And Temporal's configuration file says:
      """toml
      [profile.pastry]
      namespace = "pastry"
      """
    When I run "marche run hello.yaml --profile pastry"
    Then the command fails with exit code 2
    And the error says:
      """
      Temporal has no namespace pastry.
      """

  Scenario: A profile that Temporal's configuration file doesn't have is an error
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
      """
    When I run "marche run hello.yaml --profile pastry"
    Then the command fails with exit code 2
    And the error says:
      """
      Temporal's configuration has no profile pastry.
      """
    And the help says:
      """
      `temporal config list` shows the profiles in temporal.toml.
      """
