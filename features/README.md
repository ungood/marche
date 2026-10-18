# marche features

These files specify marche's own behavior: its CLI, cancellation, the parallel `for` extension, and crash recovery. cucumber-rs runs them as integration tests against the real binary, together with the Open Workflow conformance kit in `vendor/ows/ctk/features`.

## Writing scenarios

- Every feature starts with a short description of the capability and its rules.
- Every scenario has exactly one `When`, and the first `Then` says whether the command succeeded or failed.
- A failing scenario spells out the error and the help that the user sees. The wording is part of the spec, so review it as user-facing text.
- Reuse the steps below. If a scenario needs a new step, add it here in the same style.

## Environment

Each scenario runs in its own temporary directory, with `XDG_STATE_HOME` pointing inside it, so tests never touch the real state directory. Scenarios that execute a workflow share one `temporal server start-dev` and one `marche worker`, started on a free port the first time a scenario needs them and stopped when the run ends. The `temporal` CLI comes from mise.

## Steps

Action:

- `I run "<command>"` runs marche in the scenario's directory.

Outcome:

- `the command succeeds`, `the command fails`
- `the output says:` with the expected text as a doc string. Each line must appear in standard output, after colors are removed and whitespace is collapsed.

## Conformance kit steps

The kit's steps are defined in its [README](https://github.com/open-workflow-specification/specification/blob/main/ctk/README.md), and `tests/steps/ows.rs` implements them. marche's own features can use them too.

- `a workflow with definition:` and `given the workflow input is:`, with YAML doc strings.
- `the workflow is executed` runs `marche run --json` and then `marche show --json`.
- `the workflow should complete`, `should fault`, `should cancel`, and `should complete with output:` or `should fault with error:`. Expected errors match when every field given appears in the actual error.
- `the workflow output should have properties '<path>', ...`, `should have a '<path>' property with value:`, `should have a '<path>' property containing <n> items`, with dotted paths.
- `<task> should run first`, `run last`, `run before <task>`, `run after <task>`, by the order tasks started.
- `<task> should complete`, `fault`, `cancel`, `complete with output:`, `fault with error:`.

A kit scenario runs only when every task type in its definition is in `SUPPORTED` in `tests/steps/ows.rs`. Add a task type there when the interpreter supports it.
