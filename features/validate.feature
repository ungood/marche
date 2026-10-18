Feature: marche validate

  `marche validate <file>` makes the checks `marche run` makes, without
  connecting to Temporal or starting anything. A definition must be YAML or
  JSON, must validate against the Open Workflow schema, and must use only what
  marche supports: its task types and settings, jq expressions in strict mode,
  `then` targets that exist, and the parallel `for` extension's settings in
  their documented shape. Every problem is reported at once, each pointing at
  its line in the file: the schema's, and marche's own in the parts the schema
  accepts. It exits with 0 when the definition can run, 1 when it
  can't, and 2 when marche can't read the file.

  Scenario: A definition marche can run is valid
    Given the file "hello.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: hello
        version: '0.1.0'
      do:
        - greet:
            set:
              name: world
      """
    When I run "marche validate hello.yaml"
    Then the command succeeds
    And the output says:
      """
      hello.yaml is valid, and marche can run it.
      """

  Scenario: Validating doesn't need a Temporal server
    Given the file "hello.json":
      """json
      {
        "document": {"dsl": "1.0.4", "namespace": "examples", "name": "hello", "version": "0.1.0"},
        "do": [{"greet": {"set": {"name": "world"}}}]
      }
      """
    When I run "marche validate hello.json --address 127.0.0.1:1"
    Then the command succeeds
    And the output says:
      """
      hello.json is valid, and marche can run it.
      """

  Scenario: A file that doesn't parse points at the mistake
    Given the file "broken.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: broken
        version: '0.1.0'
      do:
        - greet:
            set: [world
      """
    When I run "marche validate broken.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      broken.yaml has a syntax error on line 8.
      """
    And the error points at line 8 of "broken.yaml"

  Scenario: A definition that isn't valid OWS points at the invalid value
    Given the file "broken.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: broken
      do:
        - greet:
            set:
              name: world
      """
    When I run "marche validate broken.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      broken.yaml isn't a valid workflow definition.
      /document: "version" is a required property
      """
    And the error points at line 1 of "broken.yaml"

  Scenario: A JSON definition's problems point at their line
    Given the file "fetch.json":
      """json
      {
        "document": {"dsl": "1.0.4", "namespace": "examples", "name": "fetch", "version": "0.1.0"},
        "do": [
          {"fetch": {"call": "http", "with": {"method": "get", "endpoint": "https://example.com"}}}
        ]
      }
      """
    When I run "marche validate fetch.json"
    Then the command fails with exit code 1
    And the error says:
      """
      fetch.json can't run on marche yet.
      /do/0/fetch is a call task, which marche doesn't support yet.
      """
    And the error points at line 4 of "fetch.json"

  Scenario: Every problem is reported, each at its line
    Given the file "later.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: later
        version: '0.1.0'
      schedule:
        every:
          hours: 1
      do:
        - fetch:
            call: http
            with:
              method: get
              endpoint: https://example.com
        - announce:
            emit:
              event:
                with:
                  source: https://example.com
                  type: com.example.done
      """
    When I run "marche validate later.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      later.yaml can't run on marche yet.
      The workflow uses schedule, which marche doesn't support yet.
      /do/0/fetch is a call task, which marche doesn't support yet.
      /do/1/announce is an emit task, which marche doesn't support yet.
      """
    And the error points at line 6 of "later.yaml"
    And the error points at line 11 of "later.yaml"
    And the error points at line 16 of "later.yaml"

  Scenario: Expressions must be jq
    Given the file "script.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: script
        version: '0.1.0'
      evaluate:
        language: javascript
      do:
        - greet:
            set:
              name: world
      """
    When I run "marche validate script.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      script.yaml can't run on marche yet.
      The workflow's expressions aren't jq, which is the only language marche supports.
      """
    And the error points at line 7 of "script.yaml"

  Scenario: Loose expressions aren't supported
    Given the file "loose.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: loose
        version: '0.1.0'
      evaluate:
        mode: loose
      do:
        - greet:
            set:
              name: world
      """
    When I run "marche validate loose.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      The workflow uses loose expressions, which marche doesn't support yet.
      """
    And the error points at line 7 of "loose.yaml"

  Scenario: then must name a task in the same list
    Given the file "jump.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: jump
        version: '0.1.0'
      do:
        - greet:
            set:
              name: world
            then: farewell
        - wave:
            set:
              hand: left
      """
    When I run "marche validate jump.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      /do/0/greet has then: farewell, but its list has no task named farewell.
      """
    And the error points at line 10 of "jump.yaml"

  Scenario: The parallel for settings must have their documented types
    Given the file "each.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: each
        version: '0.1.0'
      do:
        - testEach:
            for:
              each: crate
              in: .crates
            metadata:
              marche:
                parallel: 'yes'
                maxConcurrency: 0
                cancelOnFailure: true
            do:
              - test:
                  set:
                    crate: ${ $crate }
      """
    When I run "marche validate each.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      /do/0/testEach sets metadata.marche.parallel to "yes", but it must be true or false.
      /do/0/testEach sets metadata.marche.maxConcurrency to 0, but it must be a positive integer.
      """
    And the error points at line 13 of "each.yaml"
    And the error points at line 14 of "each.yaml"

  Scenario: A setting the parallel for extension doesn't have is rejected
    Given the file "each.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: each
        version: '0.1.0'
      do:
        - testEach:
            for:
              each: crate
              in: .crates
            metadata:
              marche:
                parallel: true
                concurrency: 4
            do:
              - test:
                  set:
                    crate: ${ $crate }
      """
    When I run "marche validate each.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      /do/0/testEach sets metadata.marche.concurrency, which isn't a marche setting. The settings are parallel, maxConcurrency and cancelOnFailure.
      """
    And the error points at line 14 of "each.yaml"

  Scenario: Only for tasks take marche settings
    Given the file "greet.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: greet
        version: '0.1.0'
      do:
        - greet:
            set:
              name: world
            metadata:
              marche:
                parallel: true
      """
    When I run "marche validate greet.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      /do/0/greet sets metadata.marche, but only for tasks take marche settings.
      """
    And the error points at line 11 of "greet.yaml"

  Scenario: A parallel for can't use while
    Given the file "each.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: each
        version: '0.1.0'
      do:
        - testEach:
            for:
              each: crate
              in: .crates
            while: .more
            metadata:
              marche:
                parallel: true
            do:
              - test:
                  set:
                    crate: ${ $crate }
      """
    When I run "marche validate each.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      /do/0/testEach can't use while, because its iterations run in parallel.
      """
    And the error points at line 11 of "each.yaml"

  Scenario: Tasks inside a parallel for can't use export
    Given the file "each.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: each
        version: '0.1.0'
      do:
        - testEach:
            for:
              each: crate
              in: .crates
            metadata:
              marche:
                parallel: true
            do:
              - test:
                  set:
                    crate: ${ $crate }
                  export:
                    as: '${ $context + {last: $crate} }'
      """
    When I run "marche validate each.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      /do/0/testEach/do/0/test can't use export, because the iterations of /do/0/testEach run in parallel.
      """
    And the error points at line 18 of "each.yaml"

  Scenario: A duration must have a fixed length
    Given the file "rest.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: rest
        version: '0.1.0'
      do:
        - rest:
            wait: P1W
      """
    When I run "marche validate rest.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      /do/0/rest sets wait to "P1W", which isn't a duration marche supports. Write days, hours, minutes and seconds, such as P1DT12H, up to 100 years: weeks, months and years have no fixed length.
      """
    And the error points at line 8 of "rest.yaml"

  Scenario: A file that doesn't exist can't be read
    When I run "marche validate missing.yaml"
    Then the command fails with exit code 2
    And the error says:
      """
      Couldn't read missing.yaml: No such file or directory (os error 2).
      """

  Scenario Outline: run: <process> isn't supported yet
    Given the file "build.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: build
        version: '0.1.0'
      do:
        - build:
            run:
              <process>: <settings>
      """
    When I run "marche validate build.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      /do/0/build is a run: <process> task, which marche doesn't support yet.
      """
    And the error points at line 9 of "build.yaml"

    Examples:
      | process   | settings                                                |
      | script    | { language: js, code: 'console.log(1)' }                |
      | container | { image: alpine }                                       |
      | workflow  | { namespace: examples, name: bake, version: '0.1.0' }   |

  Scenario: A step that doesn't wait for its process is rejected
    Given the file "detach.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: detach
        version: '0.1.0'
      do:
        - serve:
            run:
              shell:
                command: sleep 60
              await: false
      """
    When I run "marche validate detach.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      /do/0/serve uses await: false, which marche doesn't support, because the process would outlive the run.
      """
    And the error points at line 11 of "detach.yaml"

  Scenario: Schema problems and marche's own are reported together
    Given the file "both.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: both
      do:
        - fetch:
            call: http
            with:
              method: get
              endpoint: https://example.com
      """
    When I run "marche validate both.yaml"
    Then the command fails with exit code 1
    And the error says:
      """
      both.yaml isn't a valid workflow definition.
      /document: "version" is a required property
      /do/0/fetch is a call task, which marche doesn't support yet.
      """
    And the error points at line 1 of "both.yaml"
    And the error points at line 7 of "both.yaml"
