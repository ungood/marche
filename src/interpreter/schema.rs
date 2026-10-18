//! `input.schema`, `output.schema` and `export.schema`: JSON Schemas, written
//! inline as `document`, that data must match.
//!
//! Validation runs in workflow code, so schemas are compiled offline: a `$ref`
//! to anything outside the document fails to compile instead of being fetched.
//! Error messages are sorted, because the order the validator finds them in
//! isn't guaranteed.

use std::fmt;

use jsonschema::ValidationError;
use jsonschema::error::ValidationErrorKind;
use serde_json::Value;

use crate::ows;

/// Why a schema can't be used.
#[derive(Debug, Clone, PartialEq)]
pub enum Unusable {
    /// Given as a `resource` rather than a `document`.
    External,
    /// In a format other than JSON Schema.
    Format(String),
    /// A `$ref` to this URI, outside the document.
    Remote(String),
    /// Not a valid JSON Schema.
    Invalid(String),
}

impl fmt::Display for Unusable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::External => write!(f, "external schemas aren't supported yet"),
            Self::Format(format) => write!(f, "the {format} format isn't supported yet"),
            Self::Remote(uri) => write!(f, "it refers to {uri}, which marche doesn't fetch"),
            Self::Invalid(message) => write!(f, "it isn't a valid JSON Schema: {message}"),
        }
    }
}

/// Compiles a schema object, `{format, document}`.
pub fn compile(schema: &Value) -> Result<jsonschema::Validator, Unusable> {
    if schema.get("resource").is_some() {
        return Err(Unusable::External);
    }
    let format = schema
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("json");
    if format != "json" && !format.starts_with("json:") {
        return Err(Unusable::Format(format.to_string()));
    }
    jsonschema::options()
        .offline()
        .build(&schema["document"])
        .map_err(|error| match error.kind() {
            ValidationErrorKind::Referencing(jsonschema::ReferencingError::Unretrievable {
                uri,
                ..
            }) => Unusable::Remote(uri.clone()),
            _ => Unusable::Invalid(located(&error)),
        })
}

/// Checks a value against the `schema` of `owner`'s `field`, such as a task's
/// `input`, if it has one.
pub fn check(owner: &Value, field: &str, value: &Value) -> Result<(), ows::Error> {
    let Some(schema) = owner.get(field).and_then(|field| field.get("schema")) else {
        return Ok(());
    };
    let what = if field == "export" { "context" } else { field };
    let validator = compile(schema).map_err(|unusable| {
        ows::Error::runtime(
            "Unusable schema",
            format!("marche can't use this {field}.schema, because {unusable}."),
        )
    })?;
    let mut errors: Vec<String> = validator
        .iter_errors(value)
        .map(|error| located(&error))
        .collect();
    if errors.is_empty() {
        return Ok(());
    }
    errors.sort();
    Err(ows::Error::validation(format!(
        "The {what} doesn't match {field}.schema: {}.",
        errors.join("; ")
    )))
}

/// The error's message, and where it is unless that's the root.
fn located(error: &ValidationError<'_>) -> String {
    let at = error.instance_path().to_string();
    if at.is_empty() {
        error.to_string()
    } else {
        format!("{error} at {at}")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn checks_values_against_inline_schemas() {
        let task = json!({"input": {"schema": {"document": {
            "type": "object",
            "properties": {"size": {"type": "integer"}},
            "required": ["order"],
        }}}});
        assert_eq!(check(&task, "input", &json!({"order": 1})), Ok(()));
        assert_eq!(check(&task, "output", &json!("anything")), Ok(()));
        let error = check(&task, "input", &json!({"size": "big"})).unwrap_err();
        assert_eq!(error.status, 400);
        assert_eq!(
            error.detail.as_deref(),
            Some(
                r#"The input doesn't match input.schema: "big" is not of type "integer" at /size; "order" is a required property."#
            )
        );
    }

    #[test]
    fn never_fetches_a_schema() {
        let remote = json!({"document": {"$ref": "https://example.com/order.json"}});
        assert_eq!(
            compile(&remote).err(),
            Some(Unusable::Remote("https://example.com/order.json".into()))
        );
        let file = json!({"document": {"$ref": "file:///etc/order.json"}});
        assert_eq!(
            compile(&file).err(),
            Some(Unusable::Remote("file:///etc/order.json".into()))
        );
        let external = json!({"resource": {"endpoint": "https://example.com/order.json"}});
        assert_eq!(compile(&external).err(), Some(Unusable::External));
        let avro = json!({"format": "avro", "document": {}});
        assert_eq!(compile(&avro).err(), Some(Unusable::Format("avro".into())));
    }

    #[test]
    fn local_refs_resolve() {
        let schema = json!({"document": {
            "$defs": {"dish": {"type": "string"}},
            "properties": {"dish": {"$ref": "#/$defs/dish"}},
        }});
        let output = json!({"output": {"schema": schema}});
        assert_eq!(check(&output, "output", &json!({"dish": "soup"})), Ok(()));
        assert!(check(&output, "output", &json!({"dish": 5})).is_err());
    }

    #[test]
    fn invalid_schemas_say_where() {
        let Err(Unusable::Invalid(message)) = compile(&json!({"document": {"type": 5}})) else {
            panic!("a type of 5 compiled");
        };
        assert!(message.ends_with("at /type"), "{message}");
    }
}
