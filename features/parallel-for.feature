Feature: Parallel for

  marche's extension runs a `for` task's iterations concurrently. It's set
  under the task's `metadata.marche`: `parallel: true` turns it on,
  `maxConcurrency` caps how many iterations run at once, and
  `cancelOnFailure`, `true` by default, decides what happens when one faults.

  Every iteration gets the task's input, with the item and index bound, and
  runs in a scope of its own. Iterations start in item order, each as soon as
  a slot is free. The task's output is an array of every iteration's output,
  in item order, whatever order they finished in.

  With `cancelOnFailure: true`, the first iteration to fault cancels the
  others, no more start, and the task faults with that error. With `false`,
  every iteration runs to the end, and the task then faults with the error of
  the lowest-indexed iteration that faulted. Cancelling the run cancels the
  running iterations.

  `marche validate` and `marche run` check the extension's settings, and that
  a parallel loop uses neither `while` nor `export` inside its iterations, as
  `validate.feature` specifies.

  Scenario: The output lists every iteration's output in item order
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: parallel-order
        version: '0.1.0'
      do:
        - cook:
            for:
              each: dish
              at: course
              in: .dishes
            metadata:
              marche:
                parallel: true
            do:
              - plate:
                  set:
                    dish: ${ $dish }
                    course: ${ $course }
                    table: ${ .table }
              - simmer:
                  run:
                    shell:
                      command: sleep "$1"
                      arguments:
                        - ${ (3 - $course) / 4 }
                  output:
                    as: ${ $input }
      """
    And given the workflow input is:
      """yaml
      table: 4
      dishes: [ soup, fish, cake ]
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      - dish: soup
        course: 0
        table: 4
      - dish: fish
        course: 1
        table: 4
      - dish: cake
        course: 2
        table: 4
      """
    And plate should run 3 times

  Scenario: Without maxConcurrency, every iteration runs at once
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: parallel-unlimited
        version: '0.1.0'
      do:
        - cook:
            for:
              at: course
              in: [ soup, fish, cake, bread ]
            metadata:
              marche:
                parallel: true
            do:
              - simmer:
                  run:
                    shell:
                      command: |
                        touch "started.$1"
                        for i in $(seq 100); do
                          if [ "$(ls started.* | wc -l)" -eq 4 ]; then
                            touch "together.$1"
                            exit 0
                          fi
                          sleep 0.1
                        done
                        exit 1
                      arguments:
                        - ${ $course }
        - count:
            run:
              shell:
                command: ls together.* | wc -l | tr -d ' '
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      '4'
      """

  Scenario: maxConcurrency caps the iterations running at once
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: parallel-limit
        version: '0.1.0'
      do:
        - cook:
            for:
              at: course
              in: [ soup, fish, cake, bread, salad, pie ]
            metadata:
              marche:
                parallel: true
                maxConcurrency: 2
            do:
              - simmer:
                  run:
                    shell:
                      command: |
                        touch "running.$1"
                        ls running.* | wc -l | tr -d ' ' >> counts
                        touch "started.$1"
                        next=$(( $1 + 1 ))
                        for i in $(seq 100); do
                          if [ "$next" -eq 6 ] || [ -e "started.$next" ]; then break; fi
                          sleep 0.1
                        done
                        rm "running.$1"
                      arguments:
                        - ${ $course }
        - count:
            run:
              shell:
                command: sort -n counts | tail -n 1
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      '2'
      """
    And simmer should run 6 times

  Scenario: The first iteration to fault cancels the others
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: parallel-cancel-on-failure
        version: '0.1.0'
      do:
        - cook:
            for:
              each: dish
              in: [ soup, burnt, fish, cake ]
            metadata:
              marche:
                parallel: true
                maxConcurrency: 3
            do:
              - simmer:
                  run:
                    shell:
                      command: |
                        if [ "$1" = burnt ]; then
                          sleep 0.5
                          echo "the $1 dish burned" >&2
                          exit 1
                        fi
                        sleep 30
                      arguments:
                        - ${ $dish }
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      title: Command failed
      instance: /do/0/cook/do/0/simmer
      """
    And simmer should cancel
    And simmer should run 3 times
    And cook should fault

  Scenario: With cancelOnFailure false, every iteration finishes and the lowest-indexed error wins
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: parallel-finish-all
        version: '0.1.0'
      do:
        - cook:
            for:
              each: dish
              in: [ soup, fish, cake ]
            metadata:
              marche:
                parallel: true
                cancelOnFailure: false
            do:
              - simmer:
                  run:
                    shell:
                      command: |
                        case "$1" in
                          soup) sleep 1; echo "the soup burned" >&2; exit 1 ;;
                          fish) echo "the fish burned" >&2; exit 1 ;;
                          cake) sleep 1.5 ;;
                        esac
                      arguments:
                        - ${ $dish }
        - after:
            set:
              unreachable: true
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      title: Command failed
      detail: |-
        The command exited with code 1. Standard error ended with:
        the soup burned
      instance: /do/0/cook/do/0/simmer
      """
    And simmer should run 3 times
    And simmer should fault

  Scenario: Cancelling a run cancels its running iterations
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: parallel-cancel
        version: '0.1.0'
      do:
        - cook:
            for:
              in: [ soup, fish ]
            metadata:
              marche:
                parallel: true
            do:
              - simmer:
                  run:
                    shell:
                      command: sleep 60
      """
    And the workflow is running its simmer task
    When the workflow is cancelled
    Then the workflow should cancel
    And simmer should cancel
    And simmer should run 2 times
    And cook should cancel
