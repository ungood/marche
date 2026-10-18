Feature: Waiting

  A `wait` task pauses the run on a durable Temporal timer, so a wait survives
  the worker restarting. The duration is an ISO 8601 duration such as `PT1M30S`,
  or an object of `days`, `hours`, `minutes`, `seconds` and `milliseconds`. The
  task's output is its input. Cancelling the run ends the wait, and the run is
  cancelled rather than faulted, so a `try` around the wait doesn't catch it.

  Scenario: A wait passes its input on
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: wait-iso
        version: '0.1.0'
      do:
        - prepare:
            set:
              dish: soup
        - rest:
            wait: PT0.5S
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      dish: soup
      """
    And rest should run after prepare

  Scenario: A wait accepts a duration object
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: wait-object
        version: '0.1.0'
      do:
        - rest:
            wait:
              milliseconds: 300
      """
    And given the workflow input is:
      """yaml
      dish: bread
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      dish: bread
      """
    And rest should complete with output:
      """yaml
      dish: bread
      """

  Scenario: Cancelling a run ends its wait
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: wait-cancel
        version: '0.1.0'
      do:
        - guard:
            try:
              - rest:
                  wait:
                    minutes: 10
            catch:
              do:
                - recover:
                    set:
                      recovered: true
      """
    And the workflow is running its rest task
    When the workflow is cancelled
    Then the workflow should cancel
    And rest should cancel
    And guard should cancel
