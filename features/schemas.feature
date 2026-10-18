Feature: Input, output and context schemas

  `input.schema`, `output.schema` and `export.schema` check data against a
  JSON Schema, on the workflow and on each task. Data that doesn't match
  faults with a `validation` error, at the point the spec's data flow says:
  input before `input.from`, output after `output.as`, and the context after
  `export.as`. Schemas are written inline as `document`, in the `json` format.

  A run must replay the same way every time, so marche never fetches a
  schema. `marche run` rejects a schema given as an external `resource`, or
  one whose `$ref` points outside the document.

  Scenario: Workflow input that doesn't match input.schema faults the run
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: workflow-input-schema
        version: '0.1.0'
      input:
        schema:
          document:
            type: object
            required: [ order ]
      do:
        - cook:
            set:
              cooked: true
      """
    And given the workflow input is:
      """yaml
      dish: soup
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/validation
      status: 400
      title: Validation failed
      detail: 'The input doesn''t match input.schema: "order" is a required property.'
      instance: /input
      """

  Scenario: A task's input.schema checks its input before input.from
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: task-input-schema
        version: '0.1.0'
      do:
        - cook:
            input:
              schema:
                document:
                  type: object
                  properties:
                    order:
                      type: object
                  required: [ order ]
              from: .order
            set:
              cooked: ${ .dish }
      """
    And given the workflow input is:
      """yaml
      order:
        dish: soup
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      cooked: soup
      """

  Scenario: A task's input that doesn't match its input.schema faults the task
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: task-input-mismatch
        version: '0.1.0'
      do:
        - cook:
            input:
              schema:
                document:
                  type: object
                  properties:
                    size:
                      type: integer
            set:
              cooked: true
      """
    And given the workflow input is:
      """yaml
      size: big
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/validation
      status: 400
      title: Validation failed
      detail: 'The input doesn''t match input.schema: "big" is not of type "integer" at /size.'
      instance: /do/0/cook
      """
    And cook should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/validation
      """

  Scenario: A task's output.schema checks its output after output.as
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: task-output-schema
        version: '0.1.0'
      do:
        - cook:
            set:
              dish: soup
            output:
              as: .dish
              schema:
                document:
                  type: string
      """
    When the workflow is executed
    Then the workflow should complete with output:
      """yaml
      soup
      """

  Scenario: A task's output that doesn't match its output.schema faults the task
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: task-output-mismatch
        version: '0.1.0'
      do:
        - cook:
            set:
              dish: soup
            output:
              schema:
                document:
                  type: string
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/validation
      status: 400
      title: Validation failed
      detail: 'The output doesn''t match output.schema: {"dish":"soup"} is not of type "string".'
      instance: /do/0/cook
      """

  Scenario: A context that doesn't match export.schema faults the task
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: export-schema
        version: '0.1.0'
      do:
        - order:
            set:
              dish: 5
            export:
              as: '$context + { dish: .dish }'
              schema:
                document:
                  type: object
                  properties:
                    dish:
                      type: string
        - serve:
            set:
              served: true
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/validation
      status: 400
      title: Validation failed
      detail: 'The context doesn''t match export.schema: 5 is not of type "string" at /dish.'
      instance: /do/0/order
      """
    And order should run last

  Scenario: Workflow output.schema checks the output after output.as
    Given a workflow with definition:
      """yaml
      document:
        dsl: '1.0.4'
        namespace: test
        name: workflow-output-schema
        version: '0.1.0'
      output:
        as: .dish
        schema:
          document:
            type: object
      do:
        - cook:
            set:
              dish: soup
      """
    When the workflow is executed
    Then the workflow should fault with error:
      """yaml
      type: https://open-workflow-specification.org/spec/1.0.0/errors/validation
      status: 400
      title: Validation failed
      detail: 'The output doesn''t match output.schema: "soup" is not of type "object".'
      instance: /output
      """

  Scenario: A schema given as an external resource is rejected before the run starts
    Given the file "order.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: order
        version: '0.1.0'
      do:
        - cook:
            input:
              schema:
                resource:
                  endpoint: https://example.com/order.json
            set:
              cooked: true
      """
    When I run "marche run order.yaml"
    Then the command fails
    And the error says:
      """
      order.yaml can't run on marche yet.
      /do/0/cook uses an external input.schema, which marche doesn't support yet.
      """

  Scenario: A schema that refers outside its document is rejected before the run starts
    Given the file "order.yaml":
      """yaml
      document:
        dsl: '1.0.4'
        namespace: examples
        name: order
        version: '0.1.0'
      output:
        schema:
          document:
            $ref: urn:example:order
      do:
        - cook:
            set:
              cooked: true
      """
    When I run "marche run order.yaml"
    Then the command fails
    And the error says:
      """
      order.yaml can't run on marche yet.
      The workflow uses an output.schema that refers to urn:example:order, which marche doesn't fetch, because a run must replay the same way every time.
      """
