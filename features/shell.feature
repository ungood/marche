Feature: Shell steps

  A `run: shell` step runs `/bin/sh -c <command> sh <arguments...>` in the
  `marche.shell` activity, so `$1`, `$2` and so on are its arguments. It runs in
  the directory `marche run` was started in, unless it sets `directory`, with
  the worker's environment plus its own `environment`, and with `stdin` as
  standard input. Its output is standard output with one trailing newline
  removed, unless `return` asks for something else. A non-zero exit code
  faults the step with a runtime error, unless `return` is `code` or `all`.
  The result keeps the last 256 KiB of each stream, and the step's log keeps
  all of it. Each run of a step, such as each iteration of a loop, has logs of
  its own. Cancelling the run stops every process the step started.

  Scenario: Arguments are positional parameters
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: arguments
        version: '0.1.0'
      do:
        - greet:
            run:
              shell:
                command: echo "$1 and $2"
                arguments:
                  - salt
                  - ${ .spice }
      """
    And given the workflow input is:
      """yaml
      spice: pepper
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      salt and pepper
      """

  Scenario: A step runs in the directory marche run was started in
    Given the file "note.txt":
      """
      mise en place
      """
    And a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: directory
        version: '0.1.0'
      do:
        - read:
            run:
              shell:
                command: cat note.txt
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      mise en place
      """

  Scenario: Directory, environment and standard input can be expressions
    Given the file "kitchen/menu.txt":
      """
      soup
      """
    And a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: environment
        version: '0.1.0'
      do:
        - order:
            run:
              shell:
                command: 'echo "$CHEF: $(cat menu.txt), $(cat)"'
                directory: ${ .room }
                environment:
                  CHEF: ${ .chef }
                stdin: ${ .side }
      """
    And given the workflow input is:
      """yaml
      room: kitchen
      chef: Remy
      side: bread
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      'Remy: soup, bread'
      """

  Scenario: A non-zero exit code faults the step with a runtime error
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: exit-code
        version: '0.1.0'
      do:
        - check:
            run:
              shell:
                command: exit 3
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/runtime
      status: 500
      title: Command failed
      detail: The command exited with code 3.
      instance: /do/0/check
      """
    And check should fault with error:
      """yaml
      status: 500
      instance: /do/0/check
      """

  Scenario: return code makes the exit code the output
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: return-code
        version: '0.1.0'
      do:
        - check:
            run:
              shell:
                command: exit 3
              return: code
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      3
      """

  Scenario: return all gives the exit code and both streams
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: return-all
        version: '0.1.0'
      do:
        - check:
            run:
              shell:
                command: echo out; echo err >&2; exit 1
              return: all
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      code: 1
      stdout: out
      stderr: err
      """

  Scenario: return stderr and return none
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: return-streams
        version: '0.1.0'
      do:
        - warn:
            run:
              shell:
                command: echo careful >&2
              return: stderr
            export:
              as: '{ warning: . }'
        - quiet:
            run:
              shell:
                command: echo ignored
              return: none
            output:
              as: '{ warning: $context.warning, quiet: . }'
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      warning: careful
      quiet: null
      """

  Scenario: A step killed by a signal exits with 128 plus the signal's number
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: signal
        version: '0.1.0'
      do:
        - check:
            run:
              shell:
                command: kill -KILL $$
              return: code
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      137
      """

  Scenario: The result keeps the end of a long stream, and the log keeps all of it
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: long-output
        version: '0.1.0'
      do:
        - count:
            run:
              shell:
                command: seq 1 100000
            output:
              as: '{ length: length, last: (split("\n") | last) }'
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      length: 262143
      last: '100000'
      """
    And count should have a standard output log of 588895 bytes

  @wip
  Scenario: Each run of a step keeps logs of its own
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: logs-per-run
        version: '0.1.0'
      do:
        - serve:
            for:
              each: dish
              in: [ soup, fish ]
            do:
              - plate:
                  run:
                    shell:
                      command: echo "$1"
                      arguments:
                        - ${ $dish }
        - cook:
            for:
              each: dish
              in: [ cake, bread, pie ]
            metadata:
              marche:
                parallel: true
            do:
              - simmer:
                  run:
                    shell:
                      command: echo "$1"
                      arguments:
                        - ${ $dish }
      """
    When the workflow is executed
    Then the workflow should complete
    And each run of plate has a standard output log of its own, saying:
      """yaml
      [ soup, fish ]
      """
    And each run of simmer has a standard output log of its own, saying:
      """yaml
      [ cake, bread, pie ]
      """

  Scenario: Cancelling a run stops the processes its step started
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: cancel
        version: '0.1.0'
      do:
        - simmer:
            run:
              shell:
                command: sleep 300 & echo $! > simmer.tmp && mv simmer.tmp simmer.pid; wait
      """
    And the workflow is running its simmer task
    And the workflow has written the file "simmer.pid"
    When the workflow is cancelled
    Then the workflow should cancel
    And simmer should cancel
    And the process whose ID is in "simmer.pid" has stopped
