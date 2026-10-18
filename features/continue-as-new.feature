Feature: Continuing as new

  Temporal limits a workflow's history, so when Temporal suggests it, a run
  continues as new: it ends its Temporal run and starts the next one with the
  same workflow ID, carrying the definition, its position, the context and
  the current data. It does that only at a point where nothing is in flight:
  between the tasks of a `do` list or a sequential `for` loop, and outside any
  `try`, `fork` or parallel `for`, which it waits to leave. The tasks around
  that point carry on in the next run without starting again.

  `marche run` follows the chain of runs to the final result, and
  `marche show` lists the tasks of every run in the chain.

  These scenarios use a worker that continues runs as new after a few tasks,
  through a test hook, rather than waiting for Temporal to suggest it.

  @wip
  Scenario: A long loop carries on in the next run
    Given the worker continues runs as new after every 3 tasks
    And a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: continue-loop
        version: '0.1.0'
      do:
        - prepare:
            set:
              served: []
              dishes: [ soup, fish, cake, bread, salad, pie ]
            export:
              as: ${ { plated: 0 } }
        - service:
            do:
              - serve:
                  for:
                    each: dish
                    in: .dishes
                  do:
                    - plate:
                        set:
                          served: ${ .served + [ $dish ] }
                        export:
                          as: ${ { plated: ($context.plated + 1) } }
        - finish:
            set:
              served: ${ .served }
              plated: ${ $context.plated }
              table: ${ $workflow.input.table }
      """
    And given the workflow input is:
      """yaml
      table: 7
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      served: [ soup, fish, cake, bread, salad, pie ]
      plated: 6
      table: 7
      """
    And the workflow should have continued as new 3 times
    And prepare should run first
    And plate should run 6 times
    And service should complete
    And finish should run last

  @wip
  Scenario: A run waits for a parallel loop to finish before continuing as new
    Given the worker continues runs as new after every 2 tasks
    And a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: continue-parallel
        version: '0.1.0'
      do:
        - cook:
            for:
              in: [ soup, fish, cake, bread ]
            metadata:
              marche:
                parallel: true
            do:
              - simmer:
                  set:
                    dish: ${ $item }
        - plate:
            set:
              count: ${ length }
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      count: 4
      """
    And the workflow should have continued as new 1 time
    And simmer should run 4 times
    And cook should run first
    And plate should run last
