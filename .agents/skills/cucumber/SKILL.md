---
name: cucumber
description: Conventions for writing Gherkin .feature files and cucumber-rs step definitions in Rust projects. Invoke when authoring or reviewing specs, scaffolding the cucumber runner, or adding step definitions.
---

# cucumber

Conventions for cucumber-rs (<https://crates.io/crates/cucumber>) in Rust projects.

## File layout

- `features/<area>.feature` — one feature file per top-level capability. Filename is kebab-case and matches the user-facing surface (subcommand, endpoint, screen).
- `tests/cucumber.rs` — single runner binary (Cargo `[[test]] name = "cucumber"`).
- `tests/steps/` — step definition modules, grouped by area to mirror the feature files.

## Gherkin style

- `Feature:` line is the user-facing capability name, followed by a short description of the capability and its rules. Every feature has one.
- One declarative `Given/When/Then` chain per scenario, with exactly one `When`. Avoid imperative `And ... And ... And ...` chains that script implementation steps. If a scenario needs a second action, split it into two scenarios.
- **Assert the outcome every time.** The first `Then` after the `When` says whether the action succeeded or failed. Everything else comes after it.
- **Spell out user-facing messages.** A failure scenario asserts the error the user sees and the help that tells them how to fix it, in full sentences. The spec is where that wording gets reviewed, so write it as carefully as the behavior.
- **Use the product's real interface.** Flags, file names and messages in scenarios follow the project's naming conventions. A scenario that uses the wrong flag name specifies the wrong product.
- **Literal values, not placeholders.** Write the URL, hash or message the user would actually see, such as `https://github.com/owner/repo.git`, and make the harness serve those values offline (for git, a test-only `url.<base>.insteadOf` pointing at local bare repos). If a value can't be predicted, make the fixtures deterministic, for example with a fixed commit date. Don't assert on formatting details when a sentence in the message says the same thing.
- **Concrete examples.** Prefer `Given the cart contains "Widget A" and "Widget B"` over `Given the cart contains <some> items`. Use `Scenario Outline` only when the variation is the test's point, for example a flag and its alias.
- **One vocabulary.** Keep the project's steps listed in one place, such as `features/README.md`, and reuse them. Refer to the same thing the same way in every scenario. A setup step and an assertion can share wording (`Given "a" is a clone of "<url>"` and `Then "a" is a clone of "<url>"`).

## Run command

`cargo test --test cucumber`

## Step definitions

- One module per feature area (`tests/steps/<area>.rs`).
- Share a single small `World` struct.
- Don't introduce fixtures until a third scenario needs the same setup. Two scenarios duplicating five lines is fine; abstracting too early hides intent.
- Step regexes match natural English — write them so the same step reads naturally in unrelated scenarios.
- Match message text after stripping colors and normalizing whitespace, so the spec checks the words rather than the terminal formatting.
