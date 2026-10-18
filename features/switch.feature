Feature: Switch

  A `switch` task checks its cases in order and follows the `then` of the
  first case whose `when` is true. A case without `when` always matches, so
  it's the default. A case's `then` is `continue`, `exit`, `end` or the name
  of a task in the same list as the switch. When no case matches, the run
  follows the switch's own `then`, which is `continue` when it has none. The
  switch's output is its input, so the next task sees the same data.

  `marche run` rejects a case whose `then` names no task in the switch's
  list, before the run starts.

  Scenario: The run follows the switch's own then when no case matches
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: switch-no-match
        version: '0.1.0'
      do:
        - checkColor:
            switch:
              - red:
                  when: .color == "red"
                  then: paintRed
        - paintBlue:
            set:
              paint: blue
            then: end
        - paintRed:
            set:
              paint: red
      """
    And given the workflow input is:
      """yaml
      color: green
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      paint: blue
      """
    And checkColor should run first
    And paintBlue should run last

  Scenario: The first matching case wins
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: switch-order
        version: '0.1.0'
      do:
        - checkSize:
            switch:
              - large:
                  when: .size > 10
                  then: serveLarge
              - medium:
                  when: .size > 5
                  then: serveMedium
        - serveMedium:
            set:
              portion: medium
            then: end
        - serveLarge:
            set:
              portion: large
      """
    And given the workflow input is:
      """yaml
      size: 12
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      portion: large
      """
    And checkSize should run first
    And serveLarge should run last

  Scenario: A case's then exit leaves the enclosing do
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: switch-exit
        version: '0.1.0'
      do:
        - course:
            do:
              - checkAppetite:
                  switch:
                    - full:
                        when: .full
                        then: exit
              - main:
                  set:
                    served: main
        - dessert:
            set:
              served: dessert
      """
    And given the workflow input is:
      """yaml
      full: true
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      served: dessert
      """
    And checkAppetite should run before dessert
    And dessert should run last

  Scenario: A case whose when fails faults the switch
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: switch-fails
        version: '0.1.0'
      do:
        - checkColor:
            switch:
              - red:
                  when: .color | ascii_downcase == "red"
                  then: end
      """
    And given the workflow input is:
      """yaml
      color: 7
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/expression
      status: 400
      instance: /do/0/checkColor
      """
    And checkColor should fault

  Scenario: A case whose then names no task in the list is rejected before the run starts
    Given the file "paint.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: paint
        version: '0.1.0'
      do:
        - checkColor:
            switch:
              - red:
                  when: .color == "red"
                  then: paintRad
        - paintRed:
            set:
              paint: red
      """
    When I run "marche run paint.yaml"
    Then the command fails
    And the error says:
      """
      paint.yaml can't run on marche yet.
      /do/0/checkColor/switch/0/red has then: paintRad, but the switch's list has no task named paintRad.
      """
