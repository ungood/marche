//! Runtime expressions: jq, evaluated by jaq.
//!
//! The interpreter runs as Temporal workflow code, so evaluation must give the
//! same result on replay. The filter context leaves out jaq's builtins that
//! read the clock, the environment, the time zone or further inputs, or that
//! write to the worker's output.

use jaq_core::load::{Arena, File, Loader, parse::Def};
use jaq_core::{Compiler, Ctx, Vars, data, unwrap_valr};
use jaq_json::{Num, Val};
use serde_json::{Map, Value};

use crate::ows;

/// Builtins that aren't deterministic, as definitions or native filters.
const IMPURE: &[&str] = &[
    "now",
    "env",
    "input",
    "inputs",
    "debug",
    "debug_empty",
    "stderr",
    "stderr_empty",
    "halt",
    "halt_error",
    "localtime",
    "strflocaltime",
];

/// The arguments the spec defines, bound as `$context`, `$input` and so on.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub context: Value,
    pub input: Value,
    pub output: Value,
    pub task: Value,
    pub workflow: Value,
    pub runtime: Value,
    /// Variables a task binds, such as `for`'s `each` and `at` or `catch`'s
    /// `as`, by name without the `$`. A later binding of a name hides an earlier one.
    pub variables: Vec<(String, Value)>,
}

const VARIABLES: [&str; 6] = [
    "$context",
    "$input",
    "$output",
    "$task",
    "$workflow",
    "$runtime",
];

impl Scope {
    /// Binds a variable, as `$name`, for expressions in this scope.
    #[allow(dead_code)] // For `for` and `catch`, still to come.
    pub fn bind(&mut self, name: &str, value: Value) {
        self.variables.retain(|(bound, _)| bound != name);
        self.variables.push((name.to_string(), value));
    }

    fn names(&self) -> Vec<String> {
        VARIABLES
            .iter()
            .map(|name| name.to_string())
            .chain(self.variables.iter().map(|(name, _)| format!("${name}")))
            .collect()
    }

    fn values(&self) -> impl Iterator<Item = &Value> {
        [
            &self.context,
            &self.input,
            &self.output,
            &self.task,
            &self.workflow,
            &self.runtime,
        ]
        .into_iter()
        .chain(self.variables.iter().map(|(_, value)| value))
    }
}

/// The jq inside `${ }`, if the string is written that way.
fn enclosed(text: &str) -> Option<&str> {
    text.trim()
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
}

/// Evaluates a field that is always an expression, such as `if` or
/// `input.from`. A string is jq with or without `${ }`; an object or array is
/// a template whose `${ }` strings are evaluated.
pub fn evaluate_field(field: &Value, input: &Value, scope: &Scope) -> Result<Value, ows::Error> {
    match field {
        Value::String(text) => evaluate(enclosed(text).unwrap_or(text), input, scope),
        other => evaluate_template(other, input, scope),
    }
}

/// Evaluates every `${ }` string in a value, leaving everything else as written.
pub fn evaluate_template(
    template: &Value,
    input: &Value,
    scope: &Scope,
) -> Result<Value, ows::Error> {
    Ok(match template {
        Value::String(text) => match enclosed(text) {
            Some(code) => evaluate(code, input, scope)?,
            None => template.clone(),
        },
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| evaluate_template(item, input, scope))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| Ok((key.clone(), evaluate_template(value, input, scope)?)))
                .collect::<Result<_, ows::Error>>()?,
        ),
        _ => template.clone(),
    })
}

/// Whether jq would treat the value as true: anything but `false` and `null`.
pub fn truthy(value: &Value) -> bool {
    !matches!(value, Value::Null | Value::Bool(false))
}

/// Runs jq on the input and returns its first output, or `null` if it has none.
pub fn evaluate(code: &str, input: &Value, scope: &Scope) -> Result<Value, ows::Error> {
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs())
        .filter(|def: &Def| !IMPURE.contains(&def.name));
    let funs = jaq_core::funs()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs())
        .filter(|(name, _, _)| !IMPURE.contains(name));

    let arena = Arena::default();
    let modules = Loader::new(defs)
        .load(&arena, File { code, path: () })
        .map_err(|errors| {
            let messages: Vec<String> = errors
                .into_iter()
                .map(|(_, error)| format!("{error:?}"))
                .collect();
            ows::Error::expression(format!(
                "`{}` isn't valid jq: {}",
                code.trim(),
                messages.join("; ")
            ))
        })?;
    let names = scope.names();
    let filter = Compiler::default()
        .with_funs(funs)
        .with_global_vars(names.iter().map(String::as_str))
        .compile(modules)
        .map_err(|errors| undefined(code, errors))?;

    let vars = Vars::new(scope.values().map(to_val));
    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, vars);
    let mut outputs = filter.id.run((ctx, to_val(input))).map(unwrap_valr);
    match outputs.next() {
        None => Ok(Value::Null),
        Some(Ok(value)) => from_val(&value).map_err(ows::Error::expression),
        Some(Err(error)) => Err(ows::Error::expression(format!(
            "`{}` failed: {error}",
            code.trim()
        ))),
    }
}

fn undefined<P>(code: &str, errors: jaq_core::compile::Errors<&str, P>) -> ows::Error {
    let names: Vec<(&str, &str)> = errors
        .iter()
        .flat_map(|(_, errors)| errors.iter())
        .map(|(name, kind)| (*name, kind.as_str()))
        .collect();
    if let Some((name, _)) = names
        .iter()
        .find(|(name, _)| IMPURE.contains(name) || *name == "$ENV")
    {
        return ows::Error::expression(format!(
            "`{name}` isn't available in marche expressions, because a run must replay the same way every time. Use $workflow.startedAt or $task.startedAt instead."
        ));
    }
    let names: Vec<String> = names
        .iter()
        .map(|(name, kind)| format!("{kind} {name}"))
        .collect();
    ows::Error::expression(format!(
        "`{}` uses the undefined {}.",
        code.trim(),
        names.join(", ")
    ))
}

fn to_val(value: &Value) -> Val {
    match value {
        Value::Null => Val::Null,
        Value::Bool(b) => Val::Bool(*b),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Val::Num(Num::from_integral(i)),
            (None, Some(f)) => Val::Num(Num::Float(f)),
            (None, None) => Val::Num(Num::from_dec_str(&n.to_string())),
        },
        Value::String(s) => Val::utf8_str(s.clone().into_bytes()),
        Value::Array(items) => items.iter().map(to_val).collect(),
        Value::Object(fields) => Val::obj(
            fields
                .iter()
                .map(|(k, v)| (Val::utf8_str(k.clone().into_bytes()), to_val(v)))
                .collect(),
        ),
    }
}

fn from_val(value: &Val) -> Result<Value, String> {
    Ok(match value {
        Val::Null => Value::Null,
        Val::Bool(b) => Value::Bool(*b),
        Val::Num(Num::Int(i)) => Value::from(*i as i64),
        Val::Num(Num::Float(f)) => {
            serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number)
        }
        Val::Num(n) => {
            let text = n.to_string();
            serde_json::from_str(&text).map_err(|_| format!("{text} isn't a JSON number"))?
        }
        Val::TStr(bytes) | Val::BStr(bytes) => {
            Value::String(String::from_utf8_lossy(bytes).into_owned())
        }
        Val::Arr(items) => Value::Array(items.iter().map(from_val).collect::<Result<_, _>>()?),
        Val::Obj(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| match k {
                    Val::TStr(key) => Ok((String::from_utf8_lossy(key).into_owned(), from_val(v)?)),
                    other => Err(format!("object keys must be strings, not {other}")),
                })
                .collect::<Result<Map<_, _>, _>>()?,
        ),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn eval(code: &str, input: Value) -> Result<Value, ows::Error> {
        evaluate(code, &input, &Scope::default())
    }

    #[test]
    fn evaluates_jq() {
        assert_eq!(eval(".a + 1", json!({"a": 1})), Ok(json!(2)));
        assert_eq!(eval(".colors + [\"red\"]", json!({})), Ok(json!(["red"])));
        assert_eq!(
            eval("{a: 1.5, b: [true, null]}", json!(null)),
            Ok(json!({"a": 1.5, "b": [true, null]}))
        );
    }

    #[test]
    fn binds_the_spec_arguments() {
        let scope = Scope {
            context: json!({"order": "soup"}),
            runtime: json!({"name": "marche"}),
            ..Scope::default()
        };
        assert_eq!(
            evaluate("[$context.order, $runtime.name]", &json!(null), &scope),
            Ok(json!(["soup", "marche"]))
        );
    }

    #[test]
    fn binds_task_variables() {
        let mut scope = Scope::default();
        scope.bind("color", json!("red"));
        scope.bind("index", json!(0));
        scope.bind("color", json!("blue"));
        assert_eq!(
            evaluate("[$color, $index]", &json!(null), &scope),
            Ok(json!(["blue", 0]))
        );
    }

    #[test]
    fn templates_evaluate_only_enclosed_strings() {
        let template =
            json!({"shape": "circle", "size": "${ .size }", "list": [".size", "${.size}"]});
        let scope = Scope::default();
        assert_eq!(
            evaluate_template(&template, &json!({"size": 6}), &scope),
            Ok(json!({"shape": "circle", "size": 6, "list": [".size", 6]}))
        );
    }

    #[test]
    fn fields_accept_jq_with_or_without_braces() {
        let scope = Scope::default();
        let input = json!({"a": {"b": 1}});
        assert_eq!(evaluate_field(&json!(".a.b"), &input, &scope), Ok(json!(1)));
        assert_eq!(
            evaluate_field(&json!("${ .a.b }"), &input, &scope),
            Ok(json!(1))
        );
    }

    #[test]
    fn impure_builtins_are_left_out() {
        for code in [
            "now",
            "env",
            "$ENV",
            "input",
            "inputs",
            "debug",
            "stderr",
            "halt",
            "halt_error",
            "localtime",
            "1 | strflocaltime(\"%Y\")",
        ] {
            let error = eval(code, json!(null)).expect_err(code);
            let detail = error.detail.unwrap();
            assert!(
                detail.contains("isn't available in marche expressions"),
                "{code}: {detail}"
            );
        }
    }

    #[test]
    fn pure_time_builtins_stay() {
        assert_eq!(
            eval("0 | todate", json!(null)),
            Ok(json!("1970-01-01T00:00:00Z"))
        );
    }

    #[test]
    fn runtime_errors_are_expression_errors() {
        let error = eval("error(\"no half\")", json!(null)).unwrap_err();
        assert_eq!(error.status, 400);
        assert!(error.detail.unwrap().contains("no half"));
    }
}
