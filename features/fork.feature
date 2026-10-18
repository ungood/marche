Feature: Fork

  A `fork` task runs its branches concurrently, each a task in a scope of its
  own, and each on the fork's input. A branch's reference is the fork's
  reference followed by `/fork/branches/<index>/<name>`.

  By default the fork's output is the array of its branches' outputs, in the
  order the branches are written, whatever order they finish in. When a branch
  faults, the fork cancels the other branches, waits for them to stop, and
  faults with that branch's error. With `compete: true`, the first branch to
  complete wins: its output is the fork's output, and the other branches are
  cancelled. A cancelled branch's shell steps are stopped, and the branch is
  reported as cancelled.

  Scenario: Branches run together, and the output keeps their order
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: fork-order
        version: '0.1.0'
      do:
        - cook:
            fork:
              branches:
                - sauce:
                    run:
                      shell:
                        command: |
                          for i in $(seq 100); do
                            [ -e rice.done ] && echo sauce && exit 0
                            sleep 0.1
                          done
                          exit 1
                - rice:
                    run:
                      shell:
                        command: touch rice.done && echo rice
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      - sauce
      - rice
      """
    And sauce should run before rice

  Scenario: A faulting branch cancels the others
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: fork-fault
        version: '0.1.0'
      do:
        - cook:
            fork:
              branches:
                - simmer:
                    run:
                      shell:
                        command: echo $$ > simmer.pid && exec sleep 30
                - burn:
                    run:
                      shell:
                        command: |
                          while [ ! -s simmer.pid ]; do sleep 0.1; done
                          exit 3
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/runtime
      status: 500
      instance: /do/0/cook/fork/branches/1/burn
      """
    And burn should fault
    And simmer should cancel
    And cook should fault
    And the process whose ID is in "simmer.pid" has exited

  Scenario: With compete, the losing branch's process is killed
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: fork-compete
        version: '0.1.0'
      do:
        - race:
            fork:
              compete: true
              branches:
                - slow:
                    run:
                      shell:
                        command: echo $$ > slow.pid && exec sleep 30
                - fast:
                    run:
                      shell:
                        command: |
                          while [ ! -s slow.pid ]; do sleep 0.1; done
                          echo fast
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      fast
      """
    And fast should complete
    And slow should cancel
    And the process whose ID is in "slow.pid" has exited
