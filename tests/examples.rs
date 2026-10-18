//! Every definition in examples/ is valid against the vendored Open Workflow
//! schema and passes `marche validate`, so the examples that docs/design.md
//! uses stay correct.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

fn schema() -> jsonschema::Validator {
    let schema: Value = serde_yaml_ng::from_str(
        &fs::read_to_string("vendor/ows/schema/workflow.yaml").expect("read the vendored schema"),
    )
    .expect("the schema is YAML");
    jsonschema::validator_for(&schema).expect("the schema compiles")
}

/// The files in examples/, in order.
fn examples() -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir("examples")
        .expect("read examples/")
        .map(|entry| entry.expect("read examples/").path())
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "examples/ is empty");
    paths
}

#[test]
fn examples_are_valid() {
    let validator = schema();
    for path in examples() {
        let definition: Value =
            serde_yaml_ng::from_str(&fs::read_to_string(&path).expect("read example"))
                .unwrap_or_else(|error| panic!("{} isn't YAML: {error}", path.display()));
        let errors: Vec<_> = validator
            .iter_errors(&definition)
            .map(|error| format!("{} at {}", error, error.instance_path()))
            .collect();
        assert!(
            errors.is_empty(),
            "{} is invalid:\n{}",
            path.display(),
            errors.join("\n")
        );
    }
}

/// marche makes its own checks too, beyond the schema's, before it runs a
/// definition.
#[test]
fn examples_pass_marche_validate() {
    for path in examples() {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_marche"))
            .arg("validate")
            .arg(&path)
            .env("NO_COLOR", "1")
            .output()
            .expect("run marche validate");
        assert!(
            output.status.success(),
            "marche validate {} failed:\n{}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
