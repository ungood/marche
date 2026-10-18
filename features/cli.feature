Feature: marche command line

  marche is one binary. Its subcommands start, inspect and cancel runs, and
  `marche worker` executes them.

  Scenario: The version flag prints the version
    When I run "marche --version"
    Then the command succeeds
    And the output says:
      """
      marche 0.1.0
      """
