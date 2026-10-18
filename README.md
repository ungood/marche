# marche

marche runs [Open Workflow Specification](https://github.com/open-workflow-specification/specification) definitions as durable workflows on [Temporal](https://temporal.io/). See [docs/design.md](docs/design.md).

marche runs every task type in the M1 milestone: `do`, `set`, `switch`, `raise`, `try`, `wait`, `for`, `fork` and `run: shell`, plus the parallel `for` extension. The design's [task types](docs/design.md#task-types) table lists what comes later.

To try it, start a Temporal dev server with a database file, so runs survive restarts:

```sh
mkdir -p ~/.local/state/marche
temporal server start-dev --db-filename ~/.local/state/marche/temporal.db
```

In another terminal, start a worker. It runs until it's interrupted, and runs need one to make progress:

```sh
marche worker
```

Then run an example:

```sh
marche run examples/hello.yaml --input '{"name": "Ada"}'
```
