# marche

`marche` is a Rust workflow engine that runs Open Workflow Specification (OWS) definitions as Temporal workflows. The design is in `docs/design.md`. Read it before changing behavior.

## Documentation

This file is for agent-only context: paths, identifiers and conventions an agent needs to act. Human-facing docs live in `docs/` and the README, and this file points to them instead of repeating them.

## Skills

Skills live in `.agents/skills/`. `.claude/skills` is a symlink to that directory for Claude Code.

- `cucumber`: conventions for `.feature` files and cucumber-rs step definitions.

## Repo paths

- `docs/design.md`: the design, and the source of truth for behavior.
- `examples/`: reference OWS definitions used by the design. Each must validate against the vendored schema.
- `features/`: marche's own Gherkin features. `features/README.md` holds the step vocabulary.
- `vendor/ows/`: the OWS JSON Schema and conformance kit, copied unchanged. Never edit these files. `vendor/ows/README.md` records the upstream commit.
- `tests/cucumber.rs`: the single cucumber-rs runner, for `features/` and the conformance kit.
- `tests/steps/`: step definitions. `ows.rs` implements the kit's steps and lists the supported task types. `services.rs` starts the Temporal dev server and worker.
- `tests/examples.rs`: validates `examples/` against the schema and with `marche validate`.
- `tests/version.rs`: checks the version `build.rs` takes from `git describe`, by including `build.rs` as a module.
- `src/main.rs`: the CLI arguments and dispatch. Each command is in `src/commands/`.
- `src/error.rs`: every error marche prints. Their wording is spec, so keep it in step with `features/`.
- `src/interpreter/`: the deterministic OWS interpreter, written against the `Host` trait. `mod.rs` walks task lists and applies data flow, each task type is a module in `tasks/`, and `tests.rs` drives it with a fake host. Scopes are cancellation tokens: a task that opens a scope runs its children under a `child_token()` and cancels it on leaving. `src/workflow.rs` is the Temporal workflow `marche.interpret` that implements `Host`, and `src/shell.rs` is the `marche.shell` activity.
- `src/expr.rs`: jq expressions through jaq, without its impure builtins. `src/definition.rs`: loading a definition and the checks `marche run` and `marche validate` make, each reported at a JSON pointer that `src/spans.rs` maps to its place in the file.
- `build.rs`: sets `MARCHE_VERSION`, the version `marche --version` and `$runtime.version` report, from `git describe`.

## Conventions

- Work is tracked in GitHub Issues on `ungood/marche`, with an `M1` milestone.
- marche is a general engine. Don't add knowledge of agents, git or Orca.
- The interpreter runs as Temporal workflow code and must be deterministic: no clocks, randomness, environment, I/O or unordered iteration outside Temporal's APIs. Side effects belong in activities.
- Behavior is specified in `features/` before it's implemented. When the interpreter supports a task type, add it to `SUPPORTED` in `tests/steps/ows.rs` so the conformance kit covers it.
- Every merge to main releases if it brings a `feat:` or `fix:` commit: `.github/workflows/release.yml` runs semantic-release (configured in `.releaserc.json`), which picks the version from the conventional commits since the last `v*` tag, then tags, builds archives for Linux (x86_64 and arm64) and macOS with `mise run dist`, and publishes them with packslip. Commit types decide the release: `fix:` is a patch, `feat:` a minor, a breaking change (`feat!:`) a major. Use `chore:`, `docs:`, `test:`, `ci:`, `build:` and the like for changes that shouldn't release. Nothing is committed back: `Cargo.toml` holds the placeholder `0.0.0-development`, and `build.rs` sets the version from `git describe`. marche isn't published to crates.io.
- Issue numbers and tracker URLs appear in commit messages and PR descriptions only, never in source, comments or `.feature` files.
- `@wip` is the only Gherkin tag in `features/`. The runner excludes `@wip` scenarios.
- mise provides the Rust toolchain, `protoc` (which the Temporal SDK needs to build), the `temporal` CLI, rumdl, actionlint with shellcheck, and the tasks: `mise run build`, `mise run test` (or `test:unit` and `test:features`), `mise run lint`, `mise run fmt`, `mise run dist`. Run `mise run lint` and `mise run test` before committing.
- Machine-specific settings go in `mise.local.toml`, which git ignores.
- Test hook: `marche worker --continue-as-new-after <n>`, a hidden flag, makes the worker continue each run as new once `<n>` tasks have finished in it, as if Temporal had suggested it. `features/continue-as-new.feature` uses it. The interpreter's continue-as-new state is in `src/interpreter/resume.rs`.
