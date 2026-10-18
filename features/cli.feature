Feature: marche command line

  marche is one binary. Its subcommands start, inspect and cancel runs, and
  `marche worker` executes them.

  marche's version comes from its release tags. A build of the tag `v0.2.0`
  says `0.2.0`, and a build of a later commit says how far it is from that
  release, as `git describe` does, such as `0.2.0-3-gabc1234`. A build without
  git history says `0.0.0-development`.

  Scenario: The version flag prints the version
    When I run "marche --version"
    Then the command succeeds
    And the output is marche's version
