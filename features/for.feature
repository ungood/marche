Feature: For

  A `for` task runs its `do` tasks once for each item of a collection, one
  iteration after another. `for.in` is an expression on the task's input that
  gives an array, or an array written inline. Each iteration sees the item as
  the variable named by `for.each`, `item` by default, and its index as the
  variable named by `for.at`, `index` by default. The first iteration's input
  is the task's input, each iteration's output is the next one's input, and
  the last iteration's output is the task's output. With no items, the output
  is the input.

  `while` is checked before each iteration, with the item and index bound, on
  the data the iteration would get. The loop stops at the first iteration for
  which it's false.

  The tasks of every iteration have the same references, such as
  `/do/0/loop/do/0/plate`, so `marche show` lists them once per iteration, in
  the order they started.

  Scenario: Each iteration's output is the next one's input
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: for-chain
        version: '0.1.0'
      do:
        - serve:
            for:
              each: dish
              at: course
              in: .dishes
            do:
              - plate:
                  set:
                    served: '${ .served + [ { dish: $dish, course: $course } ] }'
      """
    And given the workflow input is:
      """yaml
      served: []
      dishes: [ soup, fish, cake ]
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      served:
        - dish: soup
          course: 0
        - dish: fish
          course: 1
        - dish: cake
          course: 2
      """
    And plate should run 3 times

  Scenario: while stops the loop
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: for-while
        version: '0.1.0'
      do:
        - fill:
            for:
              in: [ 4, 3, 5, 2 ]
            while: ${ .total + $item <= 10 }
            do:
              - add:
                  set:
                    total: ${ .total + $item }
                    last: ${ $index }
      """
    And given the workflow input is:
      """yaml
      total: 0
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      total: 7
      last: 1
      """
    And add should run 2 times

  Scenario: A loop with no items passes its input on
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: for-empty
        version: '0.1.0'
      do:
        - serve:
            for:
              in: .dishes
            do:
              - plate:
                  set:
                    plated: true
      """
    And given the workflow input is:
      """yaml
      dishes: []
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      dishes: []
      """

  Scenario: A collection that isn't an array faults the task
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: for-not-array
        version: '0.1.0'
      do:
        - serve:
            for:
              in: .dishes
            do:
              - plate:
                  set:
                    plated: true
      """
    And given the workflow input is:
      """yaml
      dishes: soup
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/expression
      status: 400
      detail: for.in must give an array, but it gave "soup".
      instance: /do/0/serve
      """

  Scenario: A failing iteration faults the loop
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: for-fault
        version: '0.1.0'
      do:
        - serve:
            for:
              each: dish
              in: [ soup, fish, cake ]
            do:
              - plate:
                  run:
                    shell:
                      command: test "$1" != fish
                      arguments:
                        - ${ $dish }
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      status: 500
      title: Command failed
      instance: /do/0/serve/do/0/plate
      """
    And plate should run 2 times
