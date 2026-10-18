//! Open Workflow Specification values that cross the workflow boundary: errors,
//! and the record of each task that `marche show` prints.

use serde::{Deserialize, Serialize};
use serde_json::Value;

const ERROR_TYPES: &str = "https://open-workflow-specification.org/spec/1.0.0/errors/";

/// An OWS error object, as a task or workflow faults with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{}", self.summary())]
pub struct Error {
    #[serde(rename = "type")]
    pub kind: String,
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
}

impl Error {
    fn standard(kind: &str, status: u16, title: &str, detail: impl Into<String>) -> Self {
        Self {
            kind: format!("{ERROR_TYPES}{kind}"),
            status,
            title: Some(title.to_string()),
            detail: Some(detail.into()),
            instance: None,
        }
    }

    pub fn runtime(title: &str, detail: impl Into<String>) -> Self {
        Self::standard("runtime", 500, title, detail)
    }

    pub fn expression(detail: impl Into<String>) -> Self {
        Self::standard("expression", 400, "Expression failed", detail)
    }

    pub fn validation(detail: impl Into<String>) -> Self {
        Self::standard("validation", 400, "Validation failed", detail)
    }

    pub fn timeout(detail: impl Into<String>) -> Self {
        Self::standard("timeout", 408, "Timed out", detail)
    }

    /// Sets the instance, unless a nested task already set it.
    pub fn at(mut self, reference: &str) -> Self {
        self.instance.get_or_insert_with(|| reference.to_string());
        self
    }

    /// The title and the detail, for messages that have room for one line.
    pub fn summary(&self) -> String {
        match (&self.title, &self.detail) {
            (Some(title), Some(detail)) => format!("{title}: {detail}"),
            (Some(text), None) | (None, Some(text)) => text.clone(),
            (None, None) => self.kind.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Completed,
    Faulted,
    Cancelled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Faulted => "faulted",
            Self::Cancelled => "cancelled",
        }
    }
}

/// One task of a run, in the order tasks started.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub reference: String,
    pub name: String,
    pub status: Status,
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub error: Option<Error>,
    /// The path of the last attempt's standard output log, for shell steps.
    #[serde(default)]
    pub log: Option<String>,
}
