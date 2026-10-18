//! Loading a definition, and the checks `marche run` and `marche validate`
//! make before starting it: the vendored OWS schema, then what marche supports.

use std::path::Path;
use std::sync::{Arc, LazyLock};

use miette::NamedSource;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{Error, Problem, SyntaxError};
use crate::interpreter::schema::{self, Unusable};
use crate::spans::Spans;

/// The most schema errors to report at once.
const MAX_ERRORS: usize = 10;

static SCHEMA: LazyLock<jsonschema::Validator> = LazyLock::new(|| {
    let schema: Value = serde_yaml_ng::from_str(include_str!("../vendor/ows/schema/workflow.yaml"))
        .expect("the vendored schema is YAML");
    jsonschema::validator_for(&schema).expect("the vendored schema compiles")
});

/// Task types the interpreter runs. `run` is checked by process.
const SUPPORTED_TASKS: &[&str] = &["do", "set", "run", "switch", "raise", "try", "wait", "fork"];
/// `do` comes last, because `for` tasks have a `do` too.
const TASK_TYPES: &[&str] = &[
    "call", "emit", "for", "fork", "listen", "raise", "run", "set", "switch", "try", "wait", "do",
];
/// Where a task holds lists of tasks, as JSON pointers relative to the task.
const TASK_LISTS: &[&str] = &["/do", "/try", "/catch/do", "/fork/branches"];

/// Settings the schema allows and the interpreter doesn't implement yet, as
/// JSON pointers relative to the workflow or a task.
const UNSUPPORTED_WORKFLOW: &[&str] = &["/use", "/schedule"];
const UNSUPPORTED_TASK: &[&str] = &[];

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
    let text = std::fs::read_to_string(path).map_err(|error| Error::Read {
        path: display.clone(),
        error,
    })?;
    check(&display, &text)
}

/// Validates and checks a definition, naming it `path` in errors.
fn check(path: &str, text: &str) -> Result<Definition, Error> {
    let json: Value = serde_yaml_ng::from_str(text).map_err(|error| syntax(path, text, error))?;

    let errors: Vec<(String, String)> = SCHEMA
        .iter_errors(&json)
        .take(MAX_ERRORS)
        .map(|error| {
            let at = error.instance_path().to_string();
            let message = format!("{}: {error}", if at.is_empty() { "/" } else { &at });
            (at, message)
        })
        .collect();
    if !errors.is_empty() {
        return Err(Error::Invalid {
            path: path.to_string(),
            problems: locate(path, text, errors),
        });
    }

    let problems = unsupported(&json);
    if !problems.is_empty() {
        return Err(Error::Unsupported {
            path: path.to_string(),
            problems: locate(path, text, problems),
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

fn syntax(path: &str, text: &str, error: serde_yaml_ng::Error) -> Error {
    let message = error.to_string();
    let Some(location) = error.location() else {
        return Error::Parse {
            path: path.to_string(),
            message,
        };
    };
    // An error at the end of the file points at its last line.
    let mut offset = location.index().min(text.trim_end().len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    // The label sits on the line, so it doesn't repeat where it is.
    let detail = match message.split_once(" at line ") {
        Some((detail, _)) => detail.to_string(),
        None => message,
    };
    Error::Syntax(Box::new(SyntaxError {
        path: path.to_string(),
        line: text[..offset].matches('\n').count() + 1,
        src: NamedSource::new(path, Arc::from(text)),
        span: (offset..offset).into(),
        detail,
    }))
}

/// Points each problem, a JSON pointer and a message, at its place in the text.
fn locate(path: &str, text: &str, problems: Vec<(String, String)>) -> Vec<Problem> {
    let spans = Spans::parse(text);
    let text: Arc<str> = Arc::from(text);
    problems
        .into_iter()
        .map(|(at, message)| Problem {
            message,
            src: NamedSource::new(path, text.clone()),
            span: spans.find(&at).into(),
        })
        .collect()
}

/// Everything in a schema-valid definition that marche can't run yet, as a
/// JSON pointer to where it is and a message.
fn unsupported(definition: &Value) -> Vec<(String, String)> {
    let mut problems = Vec::new();
    for pointer in UNSUPPORTED_WORKFLOW {
        if definition.pointer(pointer).is_some() {
            problems.push((
                pointer.to_string(),
                format!(
                    "The workflow uses {}, which marche doesn't support yet.",
                    setting(pointer)
                ),
            ));
        }
    }
    check_data(definition, "", "The workflow", &mut problems);
    let evaluate = &definition["evaluate"];
    if evaluate["language"]
        .as_str()
        .is_some_and(|language| language != "jq")
    {
        problems.push((
            "/evaluate/language".into(),
            "The workflow's expressions aren't jq, which is the only language marche supports."
                .into(),
        ));
    }
    if evaluate["mode"] == "loose" {
        problems.push((
            "/evaluate/mode".into(),
            "The workflow uses loose expressions, which marche doesn't support yet.".into(),
        ));
    }
    check_list(&definition["do"], "/do", None, &mut problems);
    problems
}

fn note(problems: &mut Vec<(String, String)>, at: String, message: String) {
    problems.push((at, message));
}

/// "a" or "an", for a task type.
fn article(kind: &str) -> &'static str {
    if kind.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

/// A pointer to a setting as the setting's dotted name, such as `input.schema`.
fn setting(pointer: &str) -> String {
    pointer[1..].replace('/', ".")
}

/// Checks a list of tasks. `parallel` is the reference of the parallel `for`
/// task the list runs in, if any.
fn check_list(
    list: &Value,
    path: &str,
    parallel: Option<&str>,
    problems: &mut Vec<(String, String)>,
) {
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
        if SUPPORTED_TASKS.contains(&kind) {
            for pointer in UNSUPPORTED_TASK {
                if task.pointer(pointer).is_some() {
                    note(
                        problems,
                        format!("{reference}{pointer}"),
                        format!(
                            "{reference} uses {}, which marche doesn't support yet.",
                            setting(pointer)
                        ),
                    );
                }
            }
        } else {
            note(
                problems,
                format!("{reference}/{kind}"),
                format!(
                    "{reference} is {} {kind} task, which marche doesn't support yet.",
                    article(kind)
                ),
            );
        }
        check_data(task, &reference, &reference, problems);
        if let Some(name) = task.pointer("/raise/error").and_then(Value::as_str) {
            note(
                problems,
                format!("{reference}/raise/error"),
                format!(
                    "{reference} raises {name} from use.errors, which marche doesn't support yet."
                ),
            );
        }
        check_try(task, &reference, &tasks, problems);
        if let Some(run) = task["run"].as_object() {
            if let Some(process) = ["container", "script", "workflow"]
                .iter()
                .find(|process| run.contains_key(**process))
            {
                note(
                    problems,
                    format!("{reference}/run/{process}"),
                    format!(
                        "{reference} is a run: {process} task, which marche doesn't support yet."
                    ),
                );
            }
            if run.get("await") == Some(&Value::Bool(false)) {
                note(
                    problems,
                    format!("{reference}/run/await"),
                    format!(
                        "{reference} uses await: false, which marche doesn't support, because the process would outlive the run."
                    ),
                );
            }
        }

        // Jumps, from the task and from its switch cases.
        let cases = task["switch"].as_array().into_iter().flatten();
        let jumps = cases
            .enumerate()
            .filter_map(|(index, case)| case.as_object().map(|case| (index, case)))
            .flat_map(|(index, case)| {
                case.iter()
                    .map(move |(name, case)| (format!("/switch/{index}/{name}"), case))
            })
            .chain([(String::new(), *task)]);
        for (at, owner) in jumps {
            if let Some(target) = owner["then"].as_str()
                && !["continue", "exit", "end"].contains(&target)
                && !tasks.iter().any(|(name, _)| *name == target)
            {
                note(
                    problems,
                    format!("{reference}{at}/then"),
                    format!(
                        "{reference}{at} has then: {target}, but its list has no task named {target}."
                    ),
                );
            }
        }

        let mut inner = parallel;
        if let Some(marche) = task.pointer("/metadata/marche")
            && check_extension(&reference, kind, marche, problems)
        {
            inner = Some(&reference);
            if task.get("while").is_some() {
                note(
                    problems,
                    format!("{reference}/while"),
                    format!("{reference} can't use while, because its iterations run in parallel."),
                );
            }
        }
        if let Some(parallel) = parallel
            && task.get("export").is_some()
        {
            note(
                problems,
                format!("{reference}/export"),
                format!(
                    "{reference} can't use export, because the iterations of {parallel} run in parallel."
                ),
            );
        }

        for pointer in TASK_LISTS {
            if let Some(list) = task.pointer(pointer) {
                check_list(list, &format!("{reference}{pointer}"), inner, problems);
            }
        }
    }
}

/// What marche doesn't support yet in a `try` task, and where its `catch.then`
/// jumps. `tasks` is the list the task is in.
fn check_try(
    task: &Value,
    reference: &str,
    tasks: &[(&String, &Value)],
    problems: &mut Vec<(String, String)>,
) {
    let catch = &task["catch"];
    if let Some(target) = catch["then"].as_str()
        && !["continue", "exit", "end"].contains(&target)
        && !tasks.iter().any(|(name, _)| *name == target)
    {
        note(
            problems,
            format!("{reference}/catch/then"),
            format!(
                "{reference} has catch.then: {target}, but its list has no task named {target}."
            ),
        );
    }
    let retry = &catch["retry"];
    if let Some(name) = retry.as_str() {
        note(
            problems,
            format!("{reference}/catch/retry"),
            format!("{reference} uses catch.retry: {name}, which marche doesn't support yet."),
        );
    }
    if retry.pointer("/limit/attempt/duration").is_some() {
        note(
            problems,
            format!("{reference}/catch/retry/limit/attempt/duration"),
            format!(
                "{reference} uses catch.retry.limit.attempt.duration, which marche doesn't support yet."
            ),
        );
    }
}

/// The schemas and timeout of the workflow or a task. `at` is its JSON
/// pointer, and `who` names it in messages.
fn check_data(owner: &Value, at: &str, who: &str, problems: &mut Vec<(String, String)>) {
    for field in ["input", "output", "export"] {
        let Some(schema) = owner.get(field).and_then(|field| field.get("schema")) else {
            continue;
        };
        let message = match schema::compile(schema) {
            Ok(_) => continue,
            Err(Unusable::External) => {
                format!("{who} uses an external {field}.schema, which marche doesn't support yet.")
            }
            Err(Unusable::Format(format)) => format!(
                "{who} uses an {field}.schema in the {format} format, which marche doesn't support yet."
            ),
            Err(Unusable::Remote(uri)) => format!(
                "{who} uses an {field}.schema that refers to {uri}, which marche doesn't fetch, because a run must replay the same way every time."
            ),
            Err(Unusable::Invalid(message)) => {
                format!("{who} uses an {field}.schema that isn't a valid JSON Schema: {message}.")
            }
        };
        note(problems, format!("{at}/{field}/schema"), message);
    }
    if let Some(name) = owner["timeout"].as_str() {
        note(
            problems,
            format!("{at}/timeout"),
            format!("{who} uses the named timeout {name}, which marche doesn't support yet."),
        );
    }
}

/// Checks the parallel `for` extension's settings, and says whether they make
/// the task's iterations run in parallel.
fn check_extension(
    reference: &str,
    kind: &str,
    marche: &Value,
    problems: &mut Vec<(String, String)>,
) -> bool {
    let at = format!("{reference}/metadata/marche");
    if kind != "for" {
        note(
            problems,
            at,
            format!("{reference} sets metadata.marche, but only for tasks take marche settings."),
        );
        return false;
    }
    let Some(settings) = marche.as_object() else {
        note(
            problems,
            at,
            format!(
                "{reference} sets metadata.marche to {marche}, but it must be an object of settings."
            ),
        );
        return false;
    };
    for (name, value) in settings {
        let expected = match name.as_str() {
            "parallel" | "cancelOnFailure" => (!value.is_boolean()).then_some("true or false"),
            "maxConcurrency" => {
                (!value.as_u64().is_some_and(|n| n > 0)).then_some("a positive integer")
            }
            _ => {
                note(
                    problems,
                    format!("{at}/{name}"),
                    format!(
                        "{reference} sets metadata.marche.{name}, which isn't a marche setting. The settings are parallel, maxConcurrency and cancelOnFailure."
                    ),
                );
                continue;
            }
        };
        if let Some(expected) = expected {
            note(
                problems,
                format!("{at}/{name}"),
                format!(
                    "{reference} sets metadata.marche.{name} to {value}, but it must be {expected}."
                ),
            );
        }
    }
    settings.get("parallel") == Some(&Value::Bool(true))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn messages(definition: &Value) -> Vec<String> {
        unsupported(definition)
            .into_iter()
            .map(|(_, message)| message)
            .collect()
    }

    #[test]
    fn reports_unsupported_tasks_and_settings() {
        let definition = json!({
            "timeout": "short",
            "do": [
                {"ready": {"listen": {"to": {"one": {"with": {"type": "ready"}}}}}},
                {"outer": {"do": [{"inner": {"set": {}, "input": {"schema": {"resource": {"endpoint": "https://example.com/order.json"}}}}}]}},
                {"jump": {"set": {}, "then": "nowhere"}},
                {"route": {"switch": [{"other": {"then": "elsewhere"}}]}},
            ]
        });
        assert_eq!(
            messages(&definition),
            [
                "The workflow uses the named timeout short, which marche doesn't support yet.",
                "/do/0/ready is a listen task, which marche doesn't support yet.",
                "/do/1/outer/do/0/inner uses an external input.schema, which marche doesn't support yet.",
                "/do/2/jump has then: nowhere, but its list has no task named nowhere.",
                "/do/3/route/switch/0/other has then: elsewhere, but its list has no task named elsewhere.",
            ]
        );
    }

    #[test]
    fn checks_the_parallel_for_extension() {
        let definition = json!({
            "do": [
                {"each": {
                    "for": {"in": ".items"},
                    "while": ".more",
                    "metadata": {"marche": {"parallel": true, "maxConcurrency": 0, "cancel": true}},
                    "do": [{"count": {"set": {}, "export": {"as": "."}}}],
                }},
                {"greet": {"set": {}, "metadata": {"marche": {"parallel": true}}}},
            ]
        });
        let problems: Vec<String> = messages(&definition)
            .into_iter()
            .filter(|message| !message.contains("is a for task"))
            .collect();
        assert_eq!(
            problems,
            [
                "/do/0/each sets metadata.marche.cancel, which isn't a marche setting. The settings are parallel, maxConcurrency and cancelOnFailure.",
                "/do/0/each sets metadata.marche.maxConcurrency to 0, but it must be a positive integer.",
                "/do/0/each can't use while, because its iterations run in parallel.",
                "/do/0/each/do/0/count can't use export, because the iterations of /do/0/each run in parallel.",
                "/do/1/greet sets metadata.marche, but only for tasks take marche settings.",
            ]
        );
    }

    #[test]
    fn problems_point_at_their_line() {
        let text = "document:\n  dsl: '1.0.4'\n  namespace: test\n  name: lines\n  version: '0.1.0'\ndo:\n  - greet:\n      set:\n        name: Ada\n  - pause:\n      listen:\n        to:\n          any: []\n";
        let Err(Error::Unsupported { problems, .. }) = check("lines.yaml", text) else {
            panic!("the definition is supported");
        };
        let offset = problems[0].span.offset();
        assert_eq!(text[..offset].matches('\n').count() + 1, 11);
    }

    #[test]
    fn syntax_errors_point_at_their_line() {
        let Err(Error::Syntax(error)) = check("broken.yaml", "do:\n  - greet:\n    set: [\n")
        else {
            panic!("the definition parses");
        };
        assert_eq!(error.line, 3);
    }

    #[test]
    fn the_examples_hello_is_supported() {
        let hello: Value = serde_yaml_ng::from_str(include_str!("../examples/hello.yaml")).unwrap();
        assert!(SCHEMA.is_valid(&hello));
        assert_eq!(unsupported(&hello), Vec::new());
    }

    #[test]
    fn checks_schemas_and_timeouts() {
        let definition = json!({
            "timeout": {"after": {"minutes": 1}},
            "input": {"schema": {"document": {"type": "object"}}},
            "output": {"schema": {"format": "avro", "document": {}}},
            "do": [
                {"cook": {
                    "set": {},
                    "timeout": {"after": "${ .after }"},
                    "input": {"schema": {"document": {"type": 5}}},
                    "export": {"schema": {"document": {"$ref": "file:///etc/context.json"}}},
                }},
            ]
        });
        assert_eq!(
            messages(&definition),
            [
                "The workflow uses an output.schema in the avro format, which marche doesn't support yet.",
                "/do/0/cook uses an input.schema that isn't a valid JSON Schema: 5 is not valid under any of the schemas listed in the 'anyOf' keyword at /type.",
                "/do/0/cook uses an export.schema that refers to file:///etc/context.json, which marche doesn't fetch, because a run must replay the same way every time.",
            ]
        );
    }

    #[test]
    fn checks_fork_branches() {
        let definition = json!({
            "do": [
                {"cook": {"fork": {"branches": [
                    {"stir": {"set": {}}},
                    {"ready": {"listen": {"to": {"one": {"with": {"type": "ready"}}}}}},
                ]}}},
            ]
        });
        assert_eq!(
            messages(&definition),
            [
                "/do/0/cook/fork/branches/1/ready is a listen task, which marche doesn't support yet."
            ]
        );
    }

    #[test]
    fn the_examples_review_loop_is_supported() {
        let review: Value =
            serde_yaml_ng::from_str(include_str!("../examples/review-loop.yaml")).unwrap();
        assert!(SCHEMA.is_valid(&review));
        assert_eq!(messages(&review), Vec::<String>::new());
    }

    #[test]
    fn the_examples_gate_is_supported() {
        let gate: Value = serde_yaml_ng::from_str(include_str!("../examples/gate.yaml")).unwrap();
        assert_eq!(messages(&gate), Vec::<String>::new());
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
            messages(&definition),
            [
                "/do/0/named uses catch.retry: patient, which marche doesn't support yet.",
                "/do/1/timed has catch.then: nowhere, but its list has no task named nowhere.",
                "/do/1/timed uses catch.retry.limit.attempt.duration, which marche doesn't support yet.",
                "/do/1/timed/try/0/ready is a listen task, which marche doesn't support yet.",
            ]
        );
    }
}
