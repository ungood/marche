# marche

marche runs [Open Workflow Specification](https://github.com/open-workflow-specification/specification) definitions as durable workflows on [Temporal](https://temporal.io/). See [docs/design.md](docs/design.md).

marche runs `do`, `set` and `run: shell` tasks so far. The M1 milestone tracks the rest. To try it, start a Temporal dev server and a worker, then run an example:

```sh
temporal server start-dev
marche worker
marche run examples/hello.yaml --input '{"name": "Ada"}'
```
