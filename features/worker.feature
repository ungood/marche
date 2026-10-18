Feature: marche worker

  `marche worker` connects to Temporal as `marche run` does, registers the
  search attributes marche sets on runs, and then runs the interpreter and the
  shell activity on the task queue `marche` until it's interrupted. With `-v`,
  it prints each shell step as it starts and finishes, after the run's ID.

  @wip
  Scenario: -v prints each step the worker runs
    Given a worker started with "marche worker -v"
    And the file "hello.yaml":
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
                command: echo "Hello, $1"
                arguments:
                  - world
      """
    When I run "marche run hello.yaml"
    Then the command succeeds
    And the worker prints:
      """
      started /do/0/say: echo "Hello, $1"
      finished /do/0/say with exit code 0
      """

  @wip
  Scenario: A worker that can't reach Temporal fails
    When I run "marche worker --address 127.0.0.1:1"
    Then the command fails with exit code 2
    And the error says:
      """
      Couldn't connect to Temporal at 127.0.0.1:1
      """
