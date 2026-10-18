# marche design

Status: draft, 2026-10-17.

marche is a general workflow engine. It runs workflows written in the [Open Workflow Specification](https://github.com/open-workflow-specification/specification) (OWS) as durable [Temporal](https://temporal.io/) workflows. A run survives crashes and reboots, waits for hours when a step asks it to, runs steps in parallel, and can be cancelled cleanly at any point.

The name comes from "ça marche!", the kitchen call that an order is accepted and should be fired. It follows `mise` (tools) and `place` (workspaces): after mise en place comes service, and marche runs it.

## Principles

- **A general engine:** marche knows nothing about agents, git, pull requests or terminals. An agent is a `run: shell` step like any other command. Anything specific to software delivery belongs in the workflows, or in separate small tools that workflows call.
- **The standard first:** definitions are plain OWS 1.0.x and validate against the official JSON Schema. marche's extensions live under task `metadata.marche`, which the schema allows, so a definition that uses them still validates, and another runtime still runs it, sequentially.
- **Durable by default:** every run is a Temporal workflow. marche keeps no state of its own beyond logs, so Temporal's guarantees are marche's guarantees.
- **Deterministic interpretation:** the interpreter runs inside Temporal workflow code, so evaluating a definition must give the same result on replay. Expressions are pure. Anything that depends on time or the environment is recorded once, when it's first needed, and replayed after that.
- **Pinned definitions:** a run carries its full definition. Editing a file never changes a run that has already started.
- **One binary:** `marche` is the CLI and the worker. Its output follows place's conventions: clap for arguments, miette for errors, console for color, and `--json` for machines.
- **Familiar names:** commands, flags and settings follow Temporal's CLI and environment variables where marche wraps a Temporal concept, and OWS where it implements a spec concept.

## Architecture

```text
workflow.yaml ──marche run──▶ validate (OWS schema + ext) ──▶ Temporal: workflow "marche.interpret"
                                hash + embed definition          │  interpreter (Rust SDK, jaq)
                                                                 ▼
                                                    activity "marche.shell" on task queue "marche"
                                                    (process group, cwd, env, stdin, heartbeat)
```

- **CLI:** reads a definition, validates it, and starts a run on the Temporal server. Other commands query, list and cancel runs.
- **Interpreter:** the Temporal workflow `marche.interpret`, written with Temporal's Rust SDK. It walks the definition's task tree, evaluates expressions with [jaq](https://github.com/01mf02/jaq), and schedules activities, timers and cancellations.
- **Shell activity:** `marche.shell` runs a `run: shell` step as a process group, with its directory, environment and standard input. It heartbeats while the process runs and kills the group when the run is cancelled.
- **Worker:** `marche worker` hosts both on the task queue `marche`.
- **Temporal server:** the operator runs `temporal server start-dev` as a user service. marche connects to it and doesn't manage it in M1.

## Concepts

- **Definition:** an OWS workflow document, in YAML or JSON. Its identity is `document.namespace`, `document.name` and `document.version`.
- **Run:** one execution of a definition: a Temporal workflow whose ID is `<namespace>.<name>-<ulid>`, such as `examples.hello-01JAB3K4Z7Y8X9W0V1T2S3R4Q5`. Commands accept the full ID or just the ULID.
- **Task:** a node in the definition, identified by its reference, the JSON Pointer the spec uses for `$task.reference` and error `instance`, such as `/do/1/say`.
- **Invoking directory:** the directory `marche run` was started in. Shell steps without a `directory` run there, and `marche list` filters by it.

## Running a workflow

`marche run <file>` does the following:

1. Parses the file and validates it against the vendored OWS schema, then checks marche's own rules: the extension's shape, supported task types, and jump targets that exist.
2. Converts it to JSON, computes the sha256 of that JSON, and generates a ULID.
3. Starts `marche.interpret` with the workflow ID `<namespace>.<name>-<ulid>` on the task queue `marche`. The workflow input holds the definition JSON, its hash, the run input, and the invoking directory.
4. Sets the search attributes `MarcheDefinition` (`<namespace>.<name>:<version>`), `MarcheDefinitionHash` and `MarcheDirectory`, so runs can be listed by definition or directory. `marche worker` registers these attributes on the server at startup if they're missing.

Because the definition travels in the workflow input, a run never reads the file again. Two runs of the same file with different contents are distinguishable by hash.

### Configuration

marche connects to Temporal the way Temporal's own CLI does: `TEMPORAL_ADDRESS` (default `localhost:7233`), `TEMPORAL_NAMESPACE` (default `default`), and the profiles in Temporal's `temporal.toml`. The global flags `--address` and `--namespace` override them. marche's own files live under the XDG state directory, `~/.local/state/marche/`.

### Temporal server

marche needs a running server. For a laptop, run the dev server with a database file so runs survive restarts:

```sh
temporal server start-dev --db-filename ~/.local/state/marche/temporal.db
```

Run it as a user service (launchd on macOS, a systemd user unit on Linux). The dev server's Web UI on port 8233 shows every run's history. Having marche install and manage that service is future work.

## CLI

Global flags: `--address <host:port>`, `--namespace <name>`, `-q/--quiet`, `-v/--verbose`. Every command that reports on runs takes `--json`.

- **`marche run <file> [--input <json>] [-d/--detach]`:** starts a run. By default it waits, prints each task as it starts and finishes on standard error, and prints the workflow output on standard output. The exit code is 0 when the run completes and 1 when it faults or is cancelled. `--detach` prints the run ID and returns at once. `--json` prints `{"run", "status", "output"}` or `{"run", "status", "error"}`, where `status` is `completed`, `faulted` or `cancelled`.
- **`marche status [<run>]`:** shows a run's state and the tasks running now. Without an argument it shows the latest run started from the current directory.
- **`marche list [--all]`:** lists recent runs started from the current directory, newest first. `--all` lists runs from every directory.
- **`marche show <run>`:** shows the run's tasks in the order they started, with each task's reference, status, output or error, and the path of its log. `--json` prints `{"run", "status", "tasks": [{"reference", "name", "status", "output", "error", "log"}]}`. The conformance harness uses this to check task order.
- **`marche cancel <run>`:** requests cancellation and waits until the run has stopped, so every process it started has exited.
- **`marche validate <file>`:** runs the checks `marche run` runs, without starting anything. Errors point at the line in the file, as place's configuration errors do.
- **`marche worker`:** runs the interpreter and the shell activity until interrupted.

`marche show` and `marche status` read task details through a workflow query, so a worker must be running to answer them. That's the normal state on a machine that runs workflows.

## Interpreter

The interpreter follows the OWS 1.0.x DSL and its reference. This section records the choices the spec leaves to runtimes, and what marche supports in M1.

### Task types

| Task | M1 | Notes |
| --- | --- | --- |
| `do` | Yes | Runs its tasks in order, following `then`. |
| `set` | Yes | |
| `switch` | Yes | Cases are checked in order. `then` can name a task in the same list, `continue`, `exit` or `end`. When no case matches, the switch's own `then` applies. |
| `raise` | Yes | Inline errors, whose `type`, `title`, `detail` and `instance` may be expressions. `instance` defaults to the task's reference. Errors from `use.errors` come with `use`. |
| `try` | Yes | `catch` with `errors.with`, `when`, `exceptWhen`, `as`, `retry` and `do`. |
| `wait` | Yes | A Temporal timer. Both duration forms are accepted. |
| `run: shell` | Yes | See [Shell steps](#shell-steps). |
| `for` | Yes | Sequential by default, or parallel with the [extension](#extension-parallel-for). |
| `fork` | Yes | Concurrent branches, with or without `compete`. |
| `listen` | M2 | With `marche signal`, delivering CloudEvents as Temporal signals. |
| `emit` | Later | |
| `call` | Later | `http`, `grpc`, `openapi`, `asyncapi`, `a2a` and `mcp`. |
| `run: script`, `container`, `workflow` | Later | `run: workflow` will start a Temporal child workflow. |

Top-level features outside M1 are `use`, `schedule` and `evaluate.language` values other than `jq`. `marche validate` rejects a definition that uses anything outside M1, naming the task and what isn't supported yet.

### Data flow

Data flows as the spec describes: `input.from`, `input.schema`, `output.as`, `output.schema` and `export.as` at both the workflow and task level, and `if` on every task. Expressions are jq, evaluated by jaq in `strict` mode, so in values such as `set`, shell arguments and `environment`, only `${ }` strings are expressions. Fields whose whole value is an expression (`if`, `input.from`, `output.as` and `export.as`) also accept jq without the `${ }`, as the conformance kit writes them.

Expressions see the spec's arguments: `$context`, `$input`, `$output`, `$workflow`, `$runtime` and `$task`, plus the `for` and `catch` variables. jaq's impure builtins are removed: `now`, `env`, `$ENV`, `input`, `inputs`, `debug`, `stderr`, `halt`, `halt_error`, and `localtime` and `strflocaltime`, which read the machine's time zone. An expression that uses one faults with an `expression` error that says why. Values that depend on the outside world come from arguments recorded once:

- `$workflow.startedAt` and `$task.startedAt` come from Temporal's workflow clock, which replays identically.
- `$runtime` is `{"name": "marche", "version": <version>, "metadata": {"directory": <invoking directory>}}`.

jaq differs from jq in a few documented corners, such as number precision and some error messages. The conformance kit is the check that the differences don't matter to OWS definitions.

### Errors

Errors are OWS error objects with `type`, `status`, `title`, `detail` and `instance`. marche raises the spec's standard types:

- A step that exits with a non-zero code raises `runtime` (status 500), with the exit code and the end of standard error in `detail`.
- An expression that fails raises `expression` (400).
- Input or output that fails its schema raises `validation` (400).
- A task or workflow `timeout` raises `timeout` (408).

`try` can catch all of these. Retry policies on `catch.retry` run in the interpreter, so a retry reruns the whole `try` block, however many tasks it holds. Delays, backoff, jitter and limits are Temporal timers and replay deterministically, with jitter drawn from Temporal's seeded random source.

Infrastructure failure is a separate layer that authors don't see. If a worker dies or a heartbeat times out, Temporal retries the activity up to three times. A step that keeps failing that way faults with a `runtime` error that says so.

### Flow and cancellation

marche follows the statechart rule: leaving a `do`, `fork` or `try` scope cancels everything started inside it. That one rule covers:

- the losing branches of a `fork` with `compete: true`;
- the other branches of a `fork` when one faults;
- the running iterations of a parallel `for` with `cancelOnFailure`;
- `marche cancel`, which leaves the outermost scope.

A cancelled shell step learns of the cancellation through its next heartbeat. The activity sends SIGTERM to the step's process group, waits ten seconds, and then sends SIGKILL. The interpreter waits for activities to confirm they've stopped before the scope finishes, so nothing a cancelled run started is still running when `marche cancel` returns.

### Shell steps

A `run: shell` step runs `/bin/sh -c <command> sh <arguments...>`, so `$1`, `$2` and so on are the step's `arguments`, as the spec's example uses them. `command`, each argument, `stdin`, `directory` and each environment value may be expressions.

- **Process:** the step runs in its own process group, so cancelling it reaches every child the command starts.
- **Directory:** `directory` if set, resolved against the invoking directory when relative; otherwise the invoking directory.
- **Environment:** the worker's environment plus `environment`. marche doesn't copy the invoking shell's environment into the run, because that would put secrets in Temporal's history. Steps that need secrets get them from the worker's environment or from a tool such as fnox inside the command.
- **Standard input:** `stdin` if set, otherwise empty.
- **Output:** `return` works as the spec says, defaulting to `stdout`. Standard output and error are text with one trailing newline removed, like `$(...)` in a shell. `all` returns `{"code", "stdout", "stderr"}`.
- **Exit code:** with `return: code` or `return: all`, the exit code is data and a non-zero code doesn't fault the step. With any other `return`, a non-zero code raises an error.
- **Crashes:** a step's process group dies with the worker, and Temporal retries the activity on the next worker once the step's heartbeats stop, after ten seconds. Long steps should therefore be idempotent. A worker that stops on SIGINT or SIGTERM kills its steps' groups itself. A worker killed with SIGKILL can't, and macOS has no parent-death signal, so each step will run under a small guard process, `marche` itself, that holds one end of a pipe from the worker and kills the step's group when the pipe closes.
- **`await: false`:** not supported in M1, because a detached process would outlive the run's cancellation rule.

### Limits

Temporal payloads are limited to 2 MB, and a workflow's history to about 50,000 events.

- Standard output and error are capped at 256 KiB each in the step's result, keeping the end of the stream. The full streams always go to `~/.local/state/marche/runs/<run>/<reference>/<attempt>.stdout` and `.stderr`, and `marche show` prints their paths.
- When Temporal suggests it, the interpreter continues as new at the next point where no tasks are in flight, carrying the definition, the current position, the context and the current data. A long sequential `for` or a `switch` loop can therefore run indefinitely.
- That state is small: the example definitions are 230 to 663 bytes as JSON, a position is a list of indexes, and the context and current data are whatever the workflow keeps, which the payload limit bounds. The task records `marche show` reads aren't carried, because they grow with every task and each holds its output, up to 512 KiB for a shell step's two streams. They stay with the Temporal run that recorded them, and `marche show` follows the chain of runs.

## Extension: parallel for

OWS has no way to run a dynamic number of tasks in parallel. `fork` needs its branches written out, and `for` runs its iterations one after another. The gap is OWS issue [#1143](https://github.com/open-workflow-specification/specification/issues/1143). marche fills it with three settings under the `for` task's `metadata.marche`. They're specified here as a short proposal that could go upstream.

```yaml
- testEachCrate:
    for:
      each: crate
      in: .crates
    metadata:
      marche:
        parallel: true
        maxConcurrency: 4
        cancelOnFailure: true
    do:
      - test:
          run:
            shell:
              command: cargo test -p "$1"
              arguments:
                - ${ $crate }
```

### Settings

| Setting | Type | Default | Meaning |
| --- | --- | --- | --- |
| `parallel` | boolean | `false` | Run iterations concurrently. The other settings apply only when this is `true`. |
| `maxConcurrency` | positive integer | no limit | The most iterations running at once. |
| `cancelOnFailure` | boolean | `true` | Cancel running iterations, and start no more, as soon as one faults. |

### Semantics

- **Input:** every iteration gets the `for` task's transformed input, with the item and index bound to the `each` and `at` variables. Iterations don't see each other's output.
- **Order:** iterations start in item order, each as soon as a slot is free.
- **Output:** an array of every iteration's output, in item order, whatever order they finished in.
- **Failure:** with `cancelOnFailure: true`, the first iteration to fault cancels the others, by the statechart rule, and the task faults with that error. With `false`, every iteration runs to the end, and the task then faults with the error of the lowest-indexed iteration that faulted.
- **Restrictions:** `while` isn't allowed, because it depends on sequential state. `export` isn't allowed inside the iterations, because concurrent writes to the context would race. `marche validate` rejects both.

### Portability

A runtime that doesn't know the extension ignores `metadata`, as the spec requires, and runs the loop sequentially. The result can differ: sequentially, each iteration's output becomes the next iteration's input, and the task's output is the last iteration's output rather than an array. Definitions that must give the same result on every runtime should keep each iteration independent of its input and shape the result with the task's `output.as`.

### Upstream

As a proposal, the same settings would move into the `for` object itself, as `for.parallel`, `for.maxConcurrency` and `for.cancelOnFailure`, which matches how Amazon States Language's `Map` and Argo's `parallelism` describe the same thing. marche would then accept both spellings.

## Temporal's Rust SDK

marche uses `temporalio-sdk` 1.x. The M1 spike settled how the design maps onto it.

- **Workflow and activity:** `marche.interpret` and `marche.shell` are the `name` overrides on `#[run]` and `#[activity]`. The workflow's input is `{definition, hash, input, directory}`, and its output is the workflow output. A faulted run fails the workflow with a non-retryable application failure whose type is the OWS error's `type` and whose details are the error object, so `marche run` can print it. A cancelled run ends the workflow as cancelled.
- **Task records:** the workflow keeps a record of each task it starts, with its status, output, error and log, and answers the query `tasks` with them. Temporal answers queries on closed runs by replaying them on a worker, so `marche show` works after a run ends, and `marche run` polls the same query to print progress.
- **The interpreter:** it's written against a small host interface (the workflow clock, task records and shell steps), which the workflow implements with its context. Unit tests drive it with a fake host. Workflow futures aren't `Send`, so the recursive walk over task lists uses local boxed futures.
- **Cancellation scopes:** `WorkflowCancellationToken` is the scope. Each `do`, `fork` and `try` takes a `child_token()` of its parent's, and passes it to the activities and timers it starts through their options' `cancellation_token`. Cancelling the token cancels everything under it, and the workflow's root token is cancelled when the run is. A token made with `WorkflowCancellationToken::new()` is detached, for cleanup that must run anyway.
- **Waiting for cancellation:** `ActivityCancellationType::WaitCancellationCompleted` makes a cancelled activity's future resolve only after the activity has confirmed it stopped, which is what `marche cancel` needs. Shell steps use it. Concurrent branches use the SDK's deterministic `select!`, `join!` and `join_all`, never tokio's or futures'.
- **Continue-as-new:** `continue_as_new_suggested()` reads the server's suggestion from the current activation, and `history_length()` gives the event count. `continue_as_new(input, options)` returns an error that the workflow propagates to end the run.
- **Randomness:** `random::<T>()` draws from Temporal's seeded generator, and `random_stream(name)` gives a named stream that other draws don't disturb. Retry jitter draws from its own stream, `marche.jitter`.
- **Time:** `workflow_time()` is the replay-safe clock behind `$workflow.startedAt` and `$task.startedAt`.
- **Shell steps:** a step heartbeats every second with a ten-second timeout, and gets three attempts for worker loss. A non-zero exit is data that the interpreter turns into an error, not an activity failure, so it's never retried as infrastructure. A command that can't start fails the activity with a non-retryable `runtime` error.
- **jaq:** the impure builtins are left out by filtering `jaq_std::defs()` and `funs()` by name before compiling. `input` and `inputs` only exist when the host supplies inputs, and `$ENV` only when it's declared, so marche declares neither.

## Examples

The definitions in [examples/](../examples) are the reference examples for this document. `mise run test` validates each one against the vendored OWS schema.

- **[hello.yaml](../examples/hello.yaml):** `set` and a shell step with arguments.
- **[parallel-for.yaml](../examples/parallel-for.yaml):** the parallel `for` extension.
- **[gate.yaml](../examples/gate.yaml):** a human gate in M1, as a polling shell step retried by `try` for up to a day.
- **[review-loop.yaml](../examples/review-loop.yaml):** a loop built from `switch` and `then`, which raises an error after three attempts.

## Testing

- **Conformance:** the OWS conformance kit is vendored in `vendor/ows/ctk/features` and runs with cucumber-rs against the real binary. Each test run starts `temporal server start-dev` on a free port, plus a `marche worker`, and stops both at the end. A kit scenario runs once every task type in its definition is supported, so each task type's work turns on its conformance scenarios.
- **marche's own behavior:** the CLI, cancellation, the parallel `for` extension, and crash recovery are specified in `features/`, in the same style as place's features.
- **Examples:** every file in `examples/` validates against the vendored schema.

## Non-goals

- Knowing about agents, git, worktrees, terminals or Orca. A wrapper that starts an agent in an Orca terminal can be its own CLI that a shell step calls.
- A web UI. Temporal's Web UI and `marche status` and `show` cover M1.
- Running the Temporal server, in M1.
- Multiple machines. One worker on one machine is the target, though nothing stops more workers joining the task queue.
- Sandboxing shell steps. The spec asks runtimes to document what they allow: marche runs any command, as the user who started the worker.

## Milestones

- **M1, a durable interpreter:** the task types above, the parallel `for` extension, the CLI, the conformance harness, and release archives. Workflows are raw OWS files run by path. Gates are `wait` plus a polling shell step.
- **M2, authoring and events:** `listen`, with `marche signal` delivering CloudEvents over Temporal signals. Pkl authoring: a Pkl schema package for OWS, `marche.pkl` at a repo root declaring named workflows, macros such as `Approval` and `ParallelForEach`, and `amends` of templates published as Pkl packages. Shared workflows, such as a standard inner workflow for every Rust repo, belong to the org that uses them, for example in its `.github` repo. marche ships no workflow templates of its own.
- **Later:** `emit`, `call`, `run: script`, `container` and `workflow`, `use`, schedules and event triggers, marche managing the Temporal server, Mermaid and DOT diagrams of a definition, and worker versioning so an interpreter upgrade can't break runs in flight.
