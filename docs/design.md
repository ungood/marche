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

marche connects to Temporal the way Temporal's own CLI does: `TEMPORAL_ADDRESS` (default `localhost:7233`), `TEMPORAL_NAMESPACE` (default `default`), and the profiles in Temporal's `temporal.toml`. The profile is `default` unless `--profile` or `TEMPORAL_PROFILE` names another, and the file is `temporalio/temporal.toml` in the user's config directory unless `--config-file` or `TEMPORAL_CONFIG_FILE` names another. The environment overrides the profile, and the global flags `--address` and `--namespace` override both. marche's own files live under the XDG state directory, `~/.local/state/marche/`.

### Temporal server

marche needs a running server. For a laptop, run the dev server with a database file so runs survive restarts:

```sh
temporal server start-dev --db-filename ~/.local/state/marche/temporal.db
```

Run it as a user service (launchd on macOS, a systemd user unit on Linux). The dev server's Web UI on port 8233 shows every run's history. Having marche install and manage that service is future work.

## CLI

Global flags: `--address <host:port>`, `--namespace <name>`, `--profile <name>`, `--config-file <path>`, `-q/--quiet`, `-v/--verbose`. Every command that reports on runs takes `--json`. `-q` leaves only a command's result, so `marche worker -q` prints nothing unless it fails, and `-v` adds detail: each finished task's output or error in `marche run`'s progress, and each shell step in `marche worker`'s output.

Exit codes are 0 for success, 1 when a run faults or is cancelled or a definition can't run, and 2 when marche can't do what it was asked: a usage error, `--input` that isn't JSON, a file it can't read, or a Temporal server it can't reach.

- **`marche run <file> [--input <json>] [-d/--detach]`:** starts a run. By default it waits, prints each task as it starts and finishes on standard error (`started /do/0/greet`, `completed /do/0/greet`), and prints the workflow output on standard output: a string as plain text, anything else as JSON. `--detach` prints the run ID and returns at once. `--json` prints `{"run", "status": "completed", "output"}`, `{"run", "status": "faulted", "error"}` or `{"run", "status": "cancelled"}`, and with `--detach`, `{"run", "status": "running"}`. While it waits, it checks every few seconds that a worker is serving the task queue `marche`, and when none is, it says so on standard error, unless `-q` is given, and goes on waiting.
- **`marche status [<run>]`:** shows a run's state, its definition, when it started, and the innermost tasks running now. Without an argument it shows the latest run started from the current directory. `--json` prints `{"run", "definition", "status", "started", "running"}`, where `running` holds task records like `marche show`'s.
- **`marche list [--all] [-n/--limit <n>]`:** lists the runs started from the current directory, newest first, a line each with the run's ID, definition, status and start time. `--all` lists runs from every directory, and `--limit` keeps only the newest, so `--limit 0` lists nothing. It says there are no runs only when none match. `--json` prints `{"runs": [{"run", "definition", "status", "started", "directory"}]}`.
- **`marche show <run>`:** shows the run's tasks in the order they started, with each task's reference, status, output or error, and the paths of its standard output and error logs. Output longer than a line is shortened unless `-v` is given. `--json` prints `{"run", "status", "tasks": [{"reference", "name", "status", "output", "error", "log"}]}`, with every field present. The conformance harness uses this to check task order.
- **`marche cancel <run>`:** requests cancellation and waits until the run has stopped, so every process it started has exited. Cancelling a run that has already ended changes nothing and succeeds. `--json` prints `{"run", "status"}` with the run's final status. Only a worker can stop a run, so when no worker is serving the run's task queue, before or while it waits, `marche cancel` says that the cancellation is requested and that the run will stop once a worker picks it up, and returns with success and the status `running`. A worker's processes die with it, so none of the run's processes are left running meanwhile.
- **`marche validate <file>`:** runs the checks `marche run` runs, without connecting to Temporal or starting anything. It reports every problem at once, each with a snippet of the file pointing at its line, as place's configuration errors do: every place the definition fails the OWS schema, then what marche's own checks find in the parts the schema accepts. A check inside a part the schema rejects would only repeat the schema's problem, so it's left out.
- **`marche worker`:** runs the interpreter and the shell activity until interrupted. It says on standard error when it's ready, unless `-q` is given. With `-v`, it prints each shell step as it starts and finishes, after its run's ID.

`marche show` and `marche status` read task details through a workflow query, so a worker must be running to answer them. That's the normal state on a machine that runs workflows. Before querying, they check that a worker is polling the run's task queue, so a missing worker is an error at once rather than a query that waits for its timeout. Temporal goes on listing a stopped worker as a poller for minutes, while an idle worker's long poll lasts about a minute, so a worker counts only when it has polled within the last two minutes. A worker that stopped more recently than that is caught by the query's own deadline: a query that no worker answers within ten seconds fails with the same error. `marche run` and `marche cancel` use the same check.

A run's status is `running`, `completed`, `faulted` or `cancelled`, the OWS names. A run terminated through Temporal, with `temporal workflow terminate` or the Web UI, is `cancelled` in every command, and `marche run` without `--json` says it was terminated. Terminating skips the run's cleanup: the interpreter never leaves its scopes, so the tasks that were running stay `running` in `marche show`, and a shell step's processes outlive the run briefly, until the step's next heartbeat finds the run gone and the guard stops them as cancellation does, which takes up to about twenty seconds.

A run's start time is its ULID's time, which stays the same when the run continues as new. `marche list` and `marche status` without a run read Temporal's visibility, through the search attribute `MarcheDirectory`. Visibility on the dev server's SQLite can't sort or match a suffix, so marche lists the matching runs, groups the Temporal runs of a continue-as-new chain under their workflow ID, and sorts them itself. A ULID is resolved the same way: marche lists the `marche.interpret` runs that started since an hour before the ULID's time and keeps those whose ID ends with it. If more than one does, the command fails and asks for the full ID. Visibility can lag a moment behind a run that has just started or ended.

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
| `wait` | Yes | A Temporal timer. Both duration forms are accepted, up to 100 years. ISO 8601 durations use days, hours, minutes and seconds, as plain decimals, because weeks, months and years have no fixed length. `marche run` rejects a duration written in the definition that isn't one, and one an expression gives faults with a `runtime` error titled `Invalid duration`. |
| `run: shell` | Yes | See [Shell steps](#shell-steps). |
| `for` | Yes | Sequential by default, or parallel with the [extension](#extension-parallel-for). `in` must give an array. `while` is checked before each iteration, with the item and index bound, on the data the iteration would get. Every iteration's tasks have the same references, so task records list them once per iteration. |
| `fork` | Yes | Concurrent branches, with or without `compete`. A branch skipped by its `if` keeps its place in the output with its input, as a skipped task's output is its input, but never wins a compete. When every branch is skipped, the fork's output is its input. A branch's `then: end` ends the workflow once the fork has finished, and its other `then` values are ignored. |
| `listen` | M2 | With `marche signal`, delivering CloudEvents as Temporal signals. |
| `emit` | Later | |
| `call` | Later | `http`, `grpc`, `openapi`, `asyncapi`, `a2a` and `mcp`. |
| `run: script`, `container`, `workflow` | Later | `run: workflow` will start a Temporal child workflow. |

Top-level features outside M1 are `use`, `schedule` and `evaluate.language` values other than `jq`. `marche validate` rejects a definition that uses anything outside M1, naming the task and what isn't supported yet.

### Data flow

Data flows as the spec describes: `input.from`, `input.schema`, `output.as`, `output.schema` and `export.as` at both the workflow and task level, and `if` on every task. Expressions are jq, evaluated by jaq in `strict` mode, so in values such as `set`, shell arguments and `environment`, only `${ }` strings are expressions. Fields whose whole value is an expression (`if`, `input.from`, `output.as` and `export.as`) also accept jq without the `${ }`, as the conformance kit writes them.

A task's `output.as` and `export.as`, a `for` task's `while`, and `catch`'s and its retry policy's `when` and `exceptWhen` see `$context` as the tasks inside them left it. `export` without `as` leaves the context as it is and checks it against `export.schema`.

In a `fork`, a branch's `export.as` replaces `$context` when that branch finishes, so tasks still running in other branches see it from then on, and when several branches export, the last to finish wins. That order replays identically but depends on timing, so a definition that needs every branch's result should read the fork's output instead.

Expressions see the spec's arguments: `$context`, `$input`, `$output`, `$workflow`, `$runtime` and `$task`, plus the `for` and `catch` variables. jaq's impure builtins are removed: `now`, `env`, `$ENV`, `input`, `inputs`, `debug`, `stderr`, `halt`, `halt_error`, and `localtime` and `strflocaltime`, which read the machine's time zone. An expression that uses one faults with an `expression` error that says why. Values that depend on the outside world come from arguments recorded once:

- `$workflow.startedAt` and `$task.startedAt` come from Temporal's workflow clock, which replays identically.
- `$runtime` is `{"name": "marche", "version": <version>, "metadata": {"directory": <invoking directory>}}`.

jaq differs from jq in a few documented corners, such as number precision and some error messages. The conformance kit is the check that the differences don't matter to OWS definitions.

Schemas are checked where the spec's data flow puts them: input against `input.schema` before `input.from`, output against `output.schema` after `output.as`, and the context against `export.schema` after `export.as`. marche supports schemas written inline as `document` in the `json` format, which is JSON Schema. Validation runs in workflow code, so marche compiles schemas offline and never fetches one: `marche run` rejects a schema given as an external `resource`, or one with a `$ref` outside its document. A `validation` error lists every mismatch, sorted, so its detail is the same on every replay.

### Errors

Errors are OWS error objects with `type`, `status`, `title`, `detail` and `instance`. marche raises the spec's standard types:

- A step that exits with a non-zero code raises `runtime` (status 500), with the exit code and the end of standard error in `detail`.
- An expression that fails raises `expression` (400).
- Input or output that fails its schema raises `validation` (400).
- A task or workflow `timeout` raises `timeout` (408).

A timeout is a Temporal timer that races the task, which runs under a scope of its own. If the timer fires first, marche cancels the task's scope, waits until everything in it has stopped, and then faults with the `timeout` error. A task that finishes in the same workflow activation as the timer fires completes, because the race checks the task first. A workflow timeout's `instance` is `/`. `timeout.after` is an inline duration or an expression, evaluated on the transformed input. A timeout named in `use.timeouts` isn't supported in M1.

`try` can catch all of these. Retry policies on `catch.retry` run in the interpreter, so a retry reruns the whole `try` block, however many tasks it holds. Delays, backoff, jitter and limits are Temporal timers and replay deterministically, with jitter drawn from Temporal's seeded random source.

The spec leaves some of `catch` to runtimes, and marche does the following:

- **Matching:** `errors.with` matches when the error has every field it lists, with equal values. `when`, `exceptWhen` and the retry policy's own `when` and `exceptWhen` see the error as `$error`, or under the name `as` gives, and so do the `catch.do` tasks.
- **Delays:** the wait before retry `n` is `delay` with `constant` backoff or none, `delay × n` with `linear`, and `delay × 2^(n-1)` with `exponential`. Jitter adds a random duration between `jitter.from` and `jitter.to`.
- **Busy retries:** a retry policy needs a `delay`, `limit.attempt.count` or `limit.duration`, and `marche run` rejects one with none of them, which would retry at once, forever. Every retry waits on a timer of at least a millisecond, even with no delay, so a block that faults without waiting for anything still yields to Temporal between attempts, rather than running until Temporal's deadlock detector stops the workflow task.
- **Limits:** `limit.attempt.count` counts every attempt, including the first. `limit.duration` stops retrying when the next attempt would start after that long since the first one started. `limit.attempt.duration` isn't supported yet, and neither are retry policies named from `use.retries`.
- **Results:** a caught error runs `catch.do` with the `try` task's input, and its output is the task's output. Without `catch.do`, the task's output is its input. When a retry policy runs out, `catch.do` handles the last error, and without `catch.do` the task faults with it, so a polling loop such as the gate example doesn't carry on as if it had succeeded.

Infrastructure failure is a separate layer that authors don't see. If a worker dies or a heartbeat times out, Temporal runs the step again, for up to three attempts in all. A step whose third attempt is lost too faults with a `runtime` error titled `Step lost`, whose detail says why and how many attempts it lost.

### Flow and cancellation

marche follows the statechart rule: leaving a `do`, `fork` or `try` scope cancels everything started inside it. That one rule covers:

- the losing branches of a `fork` with `compete: true`;
- the other branches of a `fork` when one faults;
- the running iterations of a parallel `for` with `cancelOnFailure`;
- `marche cancel`, which leaves the outermost scope.

Among concurrent branches, the first to fault or complete is the first whose result the interpreter sees, in the order Temporal delivers them, so it's the same on replay. Cancelled branches and the tasks they were running are recorded as `cancelled`.

A cancelled shell step learns of the cancellation through its next heartbeat. Steps heartbeat every second, and the worker sends their heartbeats to Temporal at most a second apart rather than holding them back for most of the heartbeat timeout, so a step learns it was cancelled within about two seconds. The activity closes the pipe to the step's guard (see [Shell steps](#shell-steps)), and the guard sends SIGTERM to the step's process group, waits up to ten seconds for every process in it to exit, and then sends SIGKILL to what's left. The activity keeps heartbeating while the guard stops the step, so a step that takes the whole grace period to stop is reported as cancelled rather than lost, and it confirms the cancellation once the guard has exited, or after fifteen seconds if something the step started still holds its output open. The interpreter waits for activities to confirm they've stopped before the scope finishes, so nothing a cancelled run started is still running when `marche cancel` returns.

### Shell steps

A `run: shell` step runs `/bin/sh -c <command> sh <arguments...>`, so `$1`, `$2` and so on are the step's `arguments`, as the spec's example uses them. `command`, each argument, `stdin`, `directory` and each environment value may be expressions.

- **Process:** the step runs in its own process group, so cancelling it reaches every child the command starts.
- **Directory:** `directory` if set, resolved against the invoking directory when relative; otherwise the invoking directory.
- **Environment:** the worker's environment plus `environment`. marche doesn't copy the invoking shell's environment into the run, because that would put secrets in Temporal's history. Steps that need secrets get them from the worker's environment or from a tool such as fnox inside the command.
- **Standard input:** `stdin` if set, otherwise empty.
- **Output:** `return` works as the spec says, defaulting to `stdout`. Standard output and error are text with one trailing newline removed, like `$(...)` in a shell. `all` returns `{"code", "stdout", "stderr"}`.
- **Exit code:** with `return: code` or `return: all`, the exit code is data and a non-zero code doesn't fault the step. With any other `return`, a non-zero code raises an error.
- **Crashes:** a step's process group dies with the worker, and Temporal retries the activity on the next worker once the step's heartbeats stop, after ten seconds. Long steps should therefore be idempotent. A worker can be killed with SIGKILL, and macOS has no parent-death signal, so each step runs under a small guard process, `marche` itself through the hidden subcommand `marche guard`. The worker starts the guard in a process group of its own, with the read end of a pipe whose write end only the worker holds. The guard starts the step in another group and waits. When the pipe closes, because the step was cancelled or the worker exited however it exited, the guard stops the step's group as cancellation does. A SIGTERM, SIGINT or SIGHUP sent to the guard stops the step the same way. The step inherits the guard's standard streams, so they pass straight through, and the guard exits with the step's exit code, or is killed by the same signal, so the activity sees the step's own status.
- **`await: false`:** not supported in M1, because a detached process would outlive the run's cancellation rule.

### Limits

Temporal payloads are limited to 2 MB, and a workflow's history to about 50,000 events.

- Standard output and error are capped at 256 KiB each in the step's result, keeping the end of the stream. The full streams always go to `~/.local/state/marche/runs/<run>/<reference>/<record>/<attempt>.stdout` and `.stderr`, and `marche show` prints their paths. `<record>` is the ID of the step's task record, which counts on across continuing as new, so each run of a step, such as each iteration of a loop, keeps logs of its own.
- When Temporal suggests it, the interpreter continues as new at the next point where no tasks are in flight, carrying the definition, the current position, the context and the current data. A long sequential `for` or a `switch` loop can therefore run indefinitely.
- Such a point is just before a task in a task list, when every task around it is a `do` or a sequential `for` without a `timeout`, and the workflow has no `timeout`. Anything else around it, such as a `try`, a `fork`, a parallel `for` or a timeout's timer, has work in flight that a new run would lose, so the run waits until it has left them. Each run also finishes at least one task before it continues as new, so a chain always makes progress.
- The position is a list of frames from the workflow's task list inwards: the index of the task each list is in, the raw and transformed input and `$task.startedAt` of each `do` or `for` task around the point, a `for` task's items and current iteration, and the input of the task to run next. The next run walks the same path down without evaluating `if`, `input.from` or `for.in` again.
- That state is small: the example definitions are 230 to 663 bytes as JSON, a position is a few frames, and the context and current data are whatever the workflow keeps, which the payload limit bounds. The task records `marche show` reads aren't carried, because they grow with every task and each holds its output, up to 512 KiB for a shell step's two streams. They stay with the Temporal run that recorded them. The next run records the tasks around its position as started again, before any other, and `marche show` and `marche run` follow the chain of runs back through each run's first event, putting those records in the place of the earlier run's records that were still running.

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
- **Flow:** `then: end` inside an iteration ends that iteration, and the workflow ends once every iteration has finished.
- **Records:** concurrent iterations run the same tasks under the same references, so the host identifies each task record by an ID it returns when the task starts, not by its reference.
- **Restrictions:** `while` isn't allowed, because it depends on sequential state. `export` isn't allowed inside the iterations, because concurrent writes to the context would race. `marche validate` rejects both.

### Portability

A runtime that doesn't know the extension ignores `metadata`, as the spec requires, and runs the loop sequentially. The result can differ: sequentially, each iteration's output becomes the next iteration's input, and the task's output is the last iteration's output rather than an array. Definitions that must give the same result on every runtime should keep each iteration independent of its input and shape the result with the task's `output.as`.

### Upstream

As a proposal, the same settings would move into the `for` object itself, as `for.parallel`, `for.maxConcurrency` and `for.cancelOnFailure`, which matches how Amazon States Language's `Map` and Argo's `parallelism` describe the same thing. marche would then accept both spellings.

## Temporal's Rust SDK

marche uses `temporalio-sdk` 1.x. The M1 spike settled how the design maps onto it.

- **Workflow and activity:** `marche.interpret` and `marche.shell` are the `name` overrides on `#[run]` and `#[activity]`. The workflow's input is `{definition, hash, input, directory}`, plus `resume`, the state above, when an earlier run continued as new, and its output is the workflow output. A faulted run fails the workflow with a non-retryable application failure whose type is the OWS error's `type` and whose details are the error object, so `marche run` can print it. A cancelled run ends the workflow as cancelled.
- **Task records:** the workflow keeps a record of each task it starts, with its status, output, error and log, and answers the query `tasks` with them. Temporal answers queries on closed runs by replaying them on a worker, so `marche show` works after a run ends, and `marche run` polls the same query to print progress.
- **The interpreter:** it's written against a small host interface (the workflow clock, task records and shell steps), which the workflow implements with its context. Unit tests drive it with a fake host. Workflow futures aren't `Send`, so the recursive walk over task lists uses local boxed futures.
- **Cancellation scopes:** `WorkflowCancellationToken` is the scope. Each `do`, `fork` and `try` takes a `child_token()` of its parent's, and passes it to the activities and timers it starts through their options' `cancellation_token`. Cancelling the token cancels everything under it, and the workflow's root token is cancelled when the run is. A token made with `WorkflowCancellationToken::new()` is detached, for cleanup that must run anyway.
- **Waiting for cancellation:** `ActivityCancellationType::WaitCancellationCompleted` makes a cancelled activity's future resolve only after the activity has confirmed it stopped, which is what `marche cancel` needs. Shell steps use it. Concurrent branches use the SDK's deterministic `select!`, `join!` and `join_all`, never tokio's or futures'.
- **Continue-as-new:** `continue_as_new_suggested()` reads the server's suggestion from the current activation, and `history_length()` gives the event count. `continue_as_new(input, options)` returns an error that the workflow propagates to end the run. The interpreter unwinds to the workflow with `Halt::ContinueAsNew` to get there. `WorkflowGetResultOptions` follows continued runs by default, so `marche run` waits for the end of the chain.
- **Randomness:** `random::<T>()` draws from Temporal's seeded generator, and `random_stream(name)` gives a named stream that other draws don't disturb. Retry jitter draws from its own stream, `marche.jitter`.
- **Time:** `workflow_time()` is the replay-safe clock behind `$workflow.startedAt` and `$task.startedAt`.
- **Shell steps:** a step heartbeats every second with a ten-second timeout, and gets three attempts for worker loss, with Temporal's default backoff of one second, doubling, between them. A non-zero exit is data that the interpreter turns into an error, not an activity failure, so it's never retried as infrastructure. A command that can't start fails the activity with a non-retryable `runtime` error.
- **jaq:** the impure builtins are left out by filtering `jaq_std::defs()` and `funs()` by name before compiling. `input` and `inputs` only exist when the host supplies inputs, and `$ENV` only when it's declared, so marche declares neither.

## Examples

The definitions in [examples/](../examples) are the reference examples for this document. `mise run test` validates each one against the vendored OWS schema.

- **[hello.yaml](../examples/hello.yaml):** `set` and a shell step with arguments.
- **[parallel-for.yaml](../examples/parallel-for.yaml):** the parallel `for` extension.
- **[gate.yaml](../examples/gate.yaml):** a human gate in M1, as a polling shell step retried by `try` for up to a day.
- **[review-loop.yaml](../examples/review-loop.yaml):** a loop built from `switch` and `then`, which raises an error after three attempts.

## Testing

- **Conformance:** the OWS conformance kit is vendored in `vendor/ows/ctk/features` and runs with cucumber-rs against the real binary. Each test run starts `temporal server start-dev` on a free port, plus a `marche worker`, and stops both at the end, or when the test process dies. A kit scenario runs once every task type in its definition is supported, so each task type's work turns on its conformance scenarios.
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
