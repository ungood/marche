Feature: Crash recovery

  A run is a Temporal workflow, so it outlives the worker that runs it. If the
  worker dies during a shell step, Temporal notices when the step's heartbeats
  stop, and runs the step again on the next worker. The run then carries on
  from where it was, and tasks that had already finished don't run again.
  Shell steps should therefore be safe to run twice. Each step runs under a
  guard process that stops the step's processes as soon as its worker dies, so
  two attempts at a step never run at once. A step whose worker is lost three
  times faults with a runtime error.

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
                command: |
                  echo started >> attempts
                  sleep 3
                  echo "$1 is ready after $(wc -l < attempts | tr -d ' ') attempts"
                arguments:
                  - ${ .dish }
      """
    And the workflow is running its simmer task
    And the workflow has written the file "attempts"
    When the worker is killed and started again
    Then the workflow should complete with output:
      """yaml
      soup is ready after 2 attempts
      """
    And prepare should run before simmer

  Scenario: A worker crash stops the step's processes before the step runs again
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: crash
        version: '0.1.0'
      do:
        - simmer:
            run:
              shell:
                command: |
                  if [ -f simmer.pid ]; then
                    if kill -0 "$(cat simmer.pid)" 2>/dev/null; then
                      echo "the first attempt is still running"
                    else
                      echo "the first attempt was stopped"
                    fi
                  else
                    sleep 60 &
                    echo $! > simmer.tmp && mv simmer.tmp simmer.pid
                    wait
                  fi
      """
    And the workflow is running its simmer task
    And the workflow has written the file "simmer.pid"
    When the worker is killed and started again
    Then the workflow should complete with output:
      """yaml
      the first attempt was stopped
      """

  Scenario: A step faults after losing its worker three times
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: lost
        version: '0.1.0'
      do:
        - simmer:
            run:
              shell:
                command: sleep 60
      """
    And the workflow is running its simmer task
    When the worker is killed and started again during each of 3 attempts
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/runtime
      status: 500
      title: Step lost
      instance: /do/0/simmer
      """
