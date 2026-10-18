//! Loading a definition, and the checks `marche run` makes before starting it:
//! the vendored OWS schema, then what marche supports.

use std::path::Path;
use std::sync::LazyLock;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::Error;

/// The most schema errors to report at once.
const MAX_ERRORS: usize = 10;

static SCHEMA: LazyLock<jsonschema::Validator> = LazyLock::new(|| {
    let schema: Value = serde_yaml_ng::from_str(include_str!("../vendor/ows/schema/workflow.yaml"))
        .expect("the vendored schema is YAML");
    jsonschema::validator_for(&schema).expect("the vendored schema compiles")
});

/// Task types the interpreter runs. `run` is checked by process.
const SUPPORTED_TASKS: &[&str] = &["do", "set", "run", "try", "wait"];
/// `do` comes last, because `for` tasks have a `do` too.
const TASK_TYPES: &[&str] = &[
    "call", "emit", "for", "fork", "listen", "raise", "run", "set", "switch", "try", "wait", "do",
];

/// Settings the schema allows and the interpreter doesn't implement yet, as
/// JSON pointers relative to the workflow or a task.
const UNSUPPORTED_WORKFLOW: &[&str] = &[
    "/use",
    "/schedule",
    "/timeout",
    "/input/schema",
    "/output/schema",
];
const UNSUPPORTED_TASK: &[&str] = &[
    "/timeout",
    "/input/schema",
    "/output/schema",
    "/export/schema",
];

#[derive(Debug, Clone)]
pub struct Definition {
    pub json: Value,
    /// The sha256 of `json`, serialized with sorted keys.
    pub hash: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
}

impl Definition {
    /// `<namespace>.<name>:<version>`, the value of `MarcheDefinition`.
    pub fn id(&self) -> String {
        format!("{}.{}:{}", self.namespace, self.name, self.version)
    }
}

/// Reads, validates and checks a definition file.
pub fn load(path: &Path) -> Result<Definition, Error> {
    let display = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|source| Error::Read {
        path: display.clone(),
        source,
    })?;
    let json: Value = serde_yaml_ng::from_str(&text).map_err(|source| Error::Parse {
        path: display.clone(),
        message: source.to_string(),
    })?;

    let errors: Vec<String> = SCHEMA
        .iter_errors(&json)
        .take(MAX_ERRORS)
        .map(|error| {
            let at = error.instance_path().to_string();
            format!("{}: {error}", if at.is_empty() { "/" } else { &at })
        })
        .collect();
    if !errors.is_empty() {
        return Err(Error::Invalid {
            path: display,
            errors,
        });
    }

    let problems = unsupported(&json);
    if !problems.is_empty() {
        return Err(Error::Unsupported {
            path: display,
            problems,
        });
    }

    let hash = Sha256::digest(serde_json::to_vec(&json).expect("JSON serializes"));
    let document = &json["document"];
    let field = |name: &str| document[name].as_str().unwrap_or_default().to_string();
    Ok(Definition {
        hash: hash.iter().map(|byte| format!("{byte:02x}")).collect(),
        namespace: field("namespace"),
        name: field("name"),
        version: field("version"),
        json,
    })
}

/// Everything in a schema-valid definition that marche can't run yet.
fn unsupported(definition: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    for pointer in UNSUPPORTED_WORKFLOW {
        if definition.pointer(pointer).is_some() {
            problems.push(format!(
                "The workflow uses {pointer}, which marche doesn't support yet."
            ));
        }
    }
    let evaluate = &definition["evaluate"];
    if evaluate["language"]
        .as_str()
        .is_some_and(|language| language != "jq")
    {
        problems.push(
            "The workflow's expressions aren't jq, which is the only language marche supports."
                .into(),
        );
    }
    if evaluate["mode"] == "loose" {
        problems
            .push("The workflow uses loose expressions, which marche doesn't support yet.".into());
    }
    check_list(&definition["do"], "/do", &mut problems);
    problems
}

fn check_list(list: &Value, path: &str, problems: &mut Vec<String>) {
    let tasks: Vec<(&String, &Value)> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .flat_map(|item| item.iter())
        .collect();
    for (index, (name, task)) in tasks.iter().enumerate() {
        let reference = format!("{path}/{index}/{name}");
        let kind = TASK_TYPES
            .iter()
            .find(|kind| task.get(**kind).is_some())
            .copied()
            .unwrap_or("unknown");
        if !SUPPORTED_TASKS.contains(&kind) {
            problems.push(format!(
                "{reference} is a {kind} task, which marche doesn't support yet."
            ));
            continue;
        }
        for pointer in UNSUPPORTED_TASK {
            if task.pointer(pointer).is_some() {
                problems.push(format!(
                    "{reference} uses {}, which marche doesn't support yet.",
                    pointer[1..].replace('/', ".")
                ));
            }
        }
        if let Some(run) = task["run"].as_object() {
            if let Some(process) = ["container", "script", "workflow"]
                .iter()
                .find(|process| run.contains_key(**process))
            {
                problems.push(format!(
                    "{reference} is a run: {process} task, which marche doesn't support yet."
                ));
            }
            if run.get("await") == Some(&Value::Bool(false)) {
                problems.push(format!("{reference} uses await: false, which marche doesn't support, because the process would outlive the run."));
            }
        }
        for (field, then) in [
            ("then", &task["then"]),
            ("catch.then", &task["catch"]["then"]),
        ] {
            if let Some(target) = then.as_str()
                && !["continue", "exit", "end"].contains(&target)
                && !tasks.iter().any(|(name, _)| *name == target)
            {
                problems.push(format!(
                    "{reference} has {field}: {target}, but its list has no task named {target}."
                ));
            }
        }
        if kind == "do" {
            check_list(&task["do"], &format!("{reference}/do"), problems);
        }
        if kind == "try" {
            check_try(task, &reference, problems);
        }
    }
}

/// What marche doesn't support yet in a `try` task, and its task lists.
fn check_try(task: &Value, reference: &str, problems: &mut Vec<String>) {
    let retry = &task["catch"]["retry"];
    if let Some(name) = retry.as_str() {
        problems.push(format!(
            "{reference} uses catch.retry: {name}, which marche doesn't support yet."
        ));
    }
    if retry.pointer("/limit/attempt/duration").is_some() {
        problems.push(format!(
            "{reference} uses catch.retry.limit.attempt.duration, which marche doesn't support yet."
        ));
    }
    check_list(&task["try"], &format!("{reference}/try"), problems);
    check_list(
        &task["catch"]["do"],
        &format!("{reference}/catch/do"),
        problems,
    );
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reports_unsupported_tasks_and_settings() {
        let definition = json!({
            "timeout": {"after": {"minutes": 1}},
            "do": [
                {"ready": {"listen": {"to": {"one": {"with": {"type": "ready"}}}}}},
                {"outer": {"do": [{"inner": {"set": {}, "timeout": {"after": {"seconds": 1}}}}]}},
                {"jump": {"set": {}, "then": "nowhere"}},
                {"loop": {"for": {"in": ".items"}, "do": []}},
            ]
        });
        assert_eq!(
            unsupported(&definition),
            [
                "The workflow uses /timeout, which marche doesn't support yet.",
                "/do/0/ready is a listen task, which marche doesn't support yet.",
                "/do/1/outer/do/0/inner uses timeout, which marche doesn't support yet.",
                "/do/2/jump has then: nowhere, but its list has no task named nowhere.",
                "/do/3/loop is a for task, which marche doesn't support yet.",
            ]
        );
    }

    #[test]
    fn the_examples_hello_is_supported() {
        let hello: Value = serde_yaml_ng::from_str(include_str!("../examples/hello.yaml")).unwrap();
        assert!(SCHEMA.is_valid(&hello));
        assert_eq!(unsupported(&hello), Vec::<String>::new());
    }

    #[test]
    fn the_examples_gate_is_supported() {
        let gate: Value = serde_yaml_ng::from_str(include_str!("../examples/gate.yaml")).unwrap();
        assert_eq!(unsupported(&gate), Vec::<String>::new());
    }

    #[test]
    fn checks_try_tasks_and_their_lists() {
        let definition = json!({
            "do": [
                {"named": {"try": [], "catch": {"retry": "patient"}}},
                {"timed": {
                    "try": [{"ready": {"listen": {"to": {}}}}],
                    "catch": {
                        "retry": {"limit": {"attempt": {"duration": "PT1M"}}},
                        "then": "nowhere",
                        "do": [{"inner": {"set": {}, "timeout": {"after": "PT1S"}}}],
                    },
                }},
            ]
        });
        assert_eq!(
            unsupported(&definition),
            [
                "/do/0/named uses catch.retry: patient, which marche doesn't support yet.",
                "/do/1/timed has catch.then: nowhere, but its list has no task named nowhere.",
                "/do/1/timed uses catch.retry.limit.attempt.duration, which marche doesn't support yet.",
                "/do/1/timed/try/0/ready is a listen task, which marche doesn't support yet.",
                "/do/1/timed/catch/do/0/inner uses timeout, which marche doesn't support yet.",
            ]
        );
    }
}
