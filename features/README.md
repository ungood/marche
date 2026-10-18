# marche features

These files specify marche's own behavior: its CLI, cancellation, the parallel `for` extension, and crash recovery. cucumber-rs runs them as integration tests against the real binary, together with the Open Workflow conformance kit in `vendor/ows/ctk/features`.

## Writing scenarios

- Every feature starts with a short description of the capability and its rules.
- Every scenario has exactly one `When`, and the first `Then` says whether the command succeeded or failed.
- A failing scenario spells out the error and the help that the user sees. The wording is part of the spec, so review it as user-facing text.
- Reuse the steps below. If a scenario needs a new step, add it here in the same style.

## Environment

Each scenario runs in its own temporary directory, with `XDG_STATE_HOME` pointing inside it, so tests never touch the real state directory. Scenarios that execute a workflow share one `temporal server start-dev` and one `marche worker`, started on a free port the first time a scenario needs them and stopped when the run ends. `I run` points marche at that server with `TEMPORAL_ADDRESS`. The `temporal` CLI comes from mise.

A scenario that kills the worker gets a server and worker of its own, so it can't disturb the scenarios running beside it.

## Steps

Setup:

- `the file "<path>":` with the content as a doc string, written in the scenario's directory.

Action:

- `I run "<command>"` runs marche in the scenario's directory. The command is split into words as a shell would, so `--input '{"name": "Ada"}'` is one argument.
- `the worker is killed and started again` kills the scenario's own worker with SIGKILL, starts a new one, and waits for the run started by `the workflow is running its <task> task`.

Outcome:

- `the command succeeds`, `the command fails`
- `the output says:` with the expected text as a doc string. Each line must appear in standard output, after colors and the margin of wrapped lines are removed and whitespace is collapsed.
- `the error says:` matches standard error the same way, and `marche prints:` matches both streams together.
- `the JSON output contains:` with a JSON doc string. Standard output must be JSON with every field given.

## Conformance kit steps

The kit's steps are defined in its [README](https://github.com/open-workflow-specification/specification/blob/main/ctk/README.md), and `tests/steps/ows.rs` implements them. marche's own features can use them too.

- `a workflow with definition:` and `given the workflow input is:`, with YAML doc strings.
- `the workflow is running its <task> task` starts the run in the background, on the scenario's own server and worker, and waits until the task has started.
- `the workflow is executed` runs `marche run --json` and then `marche show --json`.
- `the workflow should complete`, `should fault`, `should cancel`, and `should complete with output:` or `should fault with error:`. Expected errors match when every field given appears in the actual error.
- `the workflow output should have properties '<path>', ...`, `should have a '<path>' property with value:`, `should have a '<path>' property containing <n> items`, with dotted paths.
- `<task> should run first`, `run last`, `run before <task>`, `run after <task>`, by the order tasks started.
- `<task> should complete`, `fault`, `cancel`, `complete with output:`, `fault with error:`.

A kit scenario runs only when every task type in its definition is in `SUPPORTED` in `tests/steps/ows.rs`. Add a task type there when the interpreter supports it.
