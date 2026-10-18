Feature: Crash recovery

  A run is a Temporal workflow, so it outlives the worker that runs it. If the
  worker dies during a shell step, Temporal notices when the step's heartbeats
  stop, and runs the step again on the next worker. The run then carries on
  from where it was, and tasks that had already finished don't run again.
  Shell steps should therefore be safe to run twice.

  Scenario: A run finishes after its worker is killed during a step
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: recovery
        version: '0.1.0'
      do:
        - prepare:
            set:
              dish: soup
        - simmer:
            run:
              shell:
                command: sleep 3 && echo "$1 is ready"
                arguments:
                  - ${ .dish }
      """
    And the workflow is running its simmer task
    When the worker is killed and started again
    Then the workflow should complete with output:
      """yaml
      soup is ready
      """
    And prepare should run before simmer
