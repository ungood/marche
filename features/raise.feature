Feature: Raise

  A `raise` task faults with the error it defines inline. `type`, `title`,
  `detail` and `instance` may be `${ }` expressions, evaluated on the task's
  input. `status` is a number as written. `instance` defaults to the raise
  task's reference.

  Errors defined in `use.errors` and raised by name aren't supported yet, so
  `marche run` rejects a raise that names one, before the run starts.

  Scenario: A raised error's fields can be expressions
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: raise-expressions
        version: '0.1.0'
      do:
        - refuse:
            raise:
              error:
                type: ${ "https://example.com/errors/" + .reason }
                status: 409
                title: ${ "No " + .dish }
                detail: The kitchen ran out.
      """
    And given the workflow input is:
      """yaml
      dish: soup
      reason: sold-out
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://example.com/errors/sold-out
      status: 409
      title: No soup
      detail: The kitchen ran out.
      instance: /do/0/refuse
      """
    And refuse should fault

  Scenario: A raised error keeps the instance it gives
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: raise-instance
        version: '0.1.0'
      do:
        - refuse:
            raise:
              error:
                type: https://example.com/errors/sold-out
                status: 409
                instance: /kitchen/soup
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      status: 409
      instance: /kitchen/soup
      """

  Scenario: marche run prints a raised error
    Given the file "soup.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: soup
        version: '0.1.0'
      do:
        - refuse:
            raise:
              error:
                type: https://example.com/errors/sold-out
                status: 409
                title: No soup
                detail: The kitchen ran out.
      """
    When I run "marche run soup.yaml"
    Then the command fails
    And the error says:
      """
      The run faulted at /do/0/refuse: No soup.
      The kitchen ran out.
      """

  Scenario: A raise that names an error from use.errors is rejected before the run starts
    Given the file "soup.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: soup
        version: '0.1.0'
      use:
        errors:
          soldOut:
            type: https://example.com/errors/sold-out
            status: 409
      do:
        - refuse:
            raise:
              error: soldOut
      """
    When I run "marche run soup.yaml"
    Then the command fails
    And the error says:
      """
      soup.yaml can't run on marche yet.
      /do/0/refuse raises soldOut from use.errors, which marche doesn't support yet.
      """
