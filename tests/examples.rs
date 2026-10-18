//! Every definition in examples/ is valid against the vendored Open Workflow
//! schema, so the examples that docs/design.md uses stay correct.

use std::fs;

use serde_json::Value;

fn schema() -> jsonschema::Validator {
    let schema: Value = serde_yaml_ng::from_str(
        &fs::read_to_string("vendor/ows/schema/workflow.yaml").expect("read the vendored schema"),
    )
    .expect("the schema is YAML");
    jsonschema::validator_for(&schema).expect("the schema compiles")
}

#[test]
fn examples_are_valid() {
    let validator = schema();
    let mut paths: Vec<_> = fs::read_dir("examples")
        .expect("read examples/")
        .map(|entry| entry.expect("read examples/").path())
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "examples/ is empty");
    for path in paths {
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
