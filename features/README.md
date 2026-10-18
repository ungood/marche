# marche features

These files specify marche's own behavior: its CLI, cancellation, the parallel `for` extension, and crash recovery. cucumber-rs runs them as integration tests against the real binary, together with the Open Workflow conformance kit in `vendor/ows/ctk/features`.

## Writing scenarios

- Every feature starts with a short description of the capability and its rules.
- Every scenario has exactly one `When`, and the first `Then` says whether the command succeeded or failed.
- A failing scenario spells out the error and the help that the user sees. The wording is part of the spec, so review it as user-facing text.
- Reuse the steps below. If a scenario needs a new step, add it here in the same style.

## Environment

Each scenario runs in its own temporary directory, and the marche commands it runs get an `XDG_STATE_HOME` inside it. Scenarios that execute a workflow share one `temporal server start-dev` and one `marche worker`, started on a free port the first time a scenario needs them and stopped when the run ends. Shell steps' logs are written by the worker, so they go under the worker's own `XDG_STATE_HOME`, a temporary directory of its own that's removed when the worker and its server stop. Either way, tests never touch the real state directory. A server that can't take its port, because another took it first, starts again on another, and the harness checks that the server it talks to is the one it started. `I run` points marche at that server with `TEMPORAL_ADDRESS`, removes any other `TEMPORAL_` variables, and sets `TEMPORAL_CONFIG_FILE` to a file in the scenario's directory, so the real Temporal configuration never applies. The `temporal` CLI comes from mise.

A scenario that kills the worker, or starts one with flags of its own, gets a server and worker of its own, so it can't disturb the scenarios running beside it. Those are stopped as soon as the scenario ends, pass or fail. Every server and worker runs in a process group of its own and dies with the test process, even when the run is interrupted, and the worker's state directory goes with them.

## Steps

Setup:

- `the file "<path>":` with the content as a doc string, written in the scenario's directory.
- `the workflow has written the file "<path>"` waits until a step of the running workflow creates the file in the scenario's directory.
- `the environment variable "<name>" is "<value>"` sets a variable for `I run`, after the ones above.
- `Temporal's configuration file says:` with TOML as a doc string, written to the file `TEMPORAL_CONFIG_FILE` names.
- `a worker started with "marche worker <flags>"` starts the scenario's own server, and a worker with those flags.
- `"<path>" has run` runs `marche run <path>` from the file's directory and waits for the run to end, however it ends, and for Temporal's visibility to list it as ended.
- `"<path>" is running its <task> task` runs `marche run <path> --detach` from the file's directory, and waits until the task has started and visibility lists the run.
- `the run "<id>" has started on a task queue no worker serves` starts a `marche.interpret` workflow with that ID on the task queue `elsewhere`, with `temporal workflow start`.
- `the worker continues runs as new after every <n> tasks` gives the scenario a server and worker of its own. The worker continues each run as new once `<n>` tasks have finished in it, through the hidden test hook `marche worker --continue-as-new-after <n>`.

Action:

- `I run "<command>"` runs marche in the scenario's directory. The command is split into words as a shell would, so `--input '{"name": "Ada"}'` is one argument.
- `the worker is killed and started again` kills the scenario's own worker with SIGKILL, starts a new one, and waits for the run started by `the workflow is running its <task> task`.
- `the worker is killed and started again during each of <n> attempts` does the same once each attempt at the running step has started, by the step's logs, and then waits for the run.
- `the workflow is cancelled` cancels the run started by `the workflow is running its <task> task` with `temporal workflow cancel`, and waits for it to stop. It stands in for `marche cancel` until that command exists.

Outcome:

- `the command succeeds`, `the command fails`, `the command fails with exit code <n>`
- `the output says:` with the expected text as a doc string. Each line must appear in standard output, after colors and miette's decoration are removed and whitespace is collapsed.
- `the output says, in order:` and `marche prints, in order:` check the same, and that each line appears after the one before it. `marche prints, in order:` reads standard error before standard output.
- `the output is marche's version`: standard output is `marche <version>`, where the version is what `git describe --tags --match 'v[0-9]*'` says for the checkout, without the `v`, or `0.0.0-development` when there's no release tag.
- `the error says:` matches standard error the same way, `the help says:` matches the error's help, and `marche prints:` matches both streams together. `marche doesn't print:` checks that no line appears in either.
- `the error points at line <n> of "<file>"` checks that one of the error's snippets shows that line.
- `the worker prints:` matches what the scenario's own worker has printed.
- `the JSON output contains:` with a JSON doc string. Standard output must be JSON with every field given. An array matches an array of the same length whose items each contain the expected item.
- `the run should end within <n> seconds` checks how long `the workflow is executed` took, or after `the workflow is cancelled`, how long the run took to stop once cancelled.
- `the process whose ID is in "<path>" has stopped` reads a process ID from a file in the scenario's directory and checks that the process has exited.
- `<task> should have a standard output log of <n> bytes` checks the size of the log `marche show` gives for the task.
- `each run of <task> has a standard output log of its own, saying:` with a YAML list as a doc string. Each run of the task, in the order they started, must have a log path of its own, and its log must hold the item, without a trailing newline.

Run IDs aren't predictable, so in commands and expected text, `$RUN` stands for the ID of the run the scenario started last, with `has run`, `is running`, `the workflow is running its <task> task` or `the workflow is executed`, and `$ULID` for that ID's ULID. Steps that act on a run find it by its ID, or by the scenario's directory, which marche records on every run, so they never touch another scenario's run.

## Conformance kit steps

The kit's steps are defined in its [README](https://github.com/open-workflow-specification/specification/blob/main/ctk/README.md), and `tests/steps/ows.rs` implements them. marche's own features can use them too.

- `a workflow with definition:` and `given the workflow input is:`, with YAML doc strings.
- `the workflow is running its <task> task` starts the run in the background, on the scenario's own server and worker, and waits until the task has started. The steps that then wait for the run to end, such as `the workflow is cancelled`, make its `marche run --json` the scenario's last command, so `the command fails with exit code <n>` checks how it exited.
- `the workflow is executed` runs `marche run --json` and then `marche show --json`.
- `the workflow should complete`, `should fault`, `should cancel`, and `should complete with output:` or `should fault with error:`. Expected errors match when every field given appears in the actual error.
- `the workflow output should have properties '<path>', ...`, `should have a '<path>' property with value:`, `should have a '<path>' property containing <n> items`, with dotted paths.
- `<task> should run first`, `run last`, `run before <task>`, `run after <task>`, by the order tasks started.
- `<task> should complete`, `fault`, `cancel`, `complete with output:`, `fault with error:`. When a task ran more than once, as a loop's tasks do, these check the first time it started.

marche adds steps of its own that read the same results:

- `<task> should run <n> times` counts the tasks of that name that started.
- `the workflow should have continued as new <n> times` counts the Temporal runs with the workflow's ID, which is one more than the times it continued as new.
- `the workflow output's '<path>' property is marche's version` compares it with the version `the output is marche's version` expects.

A kit scenario runs only when every task type in its definition is in `SUPPORTED` in `tests/steps/ows.rs`. Add a task type there when the interpreter supports it. A `run` task's type is its process, as in `run: shell`. marche's own scenarios always run, and the test run fails if one of them uses a task type that isn't in `SUPPORTED`, since that means the filter misreads definitions.
