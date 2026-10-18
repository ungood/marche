//! Every error marche prints. The wording is spec: keep it in step with
//! `features/`.

use std::process::ExitCode;
use std::sync::Arc;

use miette::{NamedSource, SourceSpan};

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum Error {
    #[error("Couldn't read {path}: {error}.")]
    Read { path: String, error: std::io::Error },

    #[error("{path} isn't YAML or JSON: {message}")]
    Parse { path: String, message: String },

    #[error(transparent)]
    #[diagnostic(transparent)]
    Syntax(Box<SyntaxError>),

    #[error("{path} isn't a valid workflow definition.")]
    Invalid {
        path: String,
        #[related]
        problems: Vec<Problem>,
    },

    #[error("{path} can't run on marche yet.")]
    Unsupported {
        path: String,
        #[related]
        problems: Vec<Problem>,
    },

    #[error("--input isn't JSON: {0}.")]
    Input(serde_json::Error),

    #[error("Couldn't connect to Temporal at {address}: {message}")]
    #[diagnostic(help(
        "Start a server with `temporal server start-dev`, or point marche at one with --address or TEMPORAL_ADDRESS."
    ))]
    Connect { address: String, message: String },

    #[error("Couldn't load the Temporal configuration: {0}")]
    Config(String),

    #[error("Temporal doesn't know the search attribute {0}.")]
    #[diagnostic(help(
        "Start `marche worker` once against this server: it registers the search attributes marche uses."
    ))]
    SearchAttributes(String),

    #[error("Couldn't start the run: {0}")]
    Start(String),

    #[error("There's no run {0}.")]
    NoRun(String),

    #[error("Couldn't read the tasks of {run}: {message}")]
    #[diagnostic(help(
        "marche show asks a worker for the run's tasks, so `marche worker` must be running."
    ))]
    Query { run: String, message: String },

    #[error("Lost track of {run}: {message}")]
    Wait { run: String, message: String },

    #[error("The run faulted at {instance}: {title}.")]
    Faulted {
        instance: String,
        title: String,
        #[help]
        detail: Option<String>,
    },

    #[error("The run was cancelled.")]
    Cancelled,

    #[error("The run was terminated.")]
    Terminated,

    #[error("The worker stopped: {0}")]
    Worker(String),
}

impl Error {
    /// 1 when the definition or the run is the problem, and 2 when marche
    /// couldn't do what it was asked, as with a usage error.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::Parse { .. }
            | Self::Syntax(_)
            | Self::Invalid { .. }
            | Self::Unsupported { .. }
            | Self::Faulted { .. }
            | Self::Cancelled
            | Self::Terminated => ExitCode::FAILURE,
            _ => ExitCode::from(2),
        }
    }
}

/// A file that doesn't parse. It holds the file's text for the snippet, so
/// it's boxed to keep `Error` small.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("{path} has a syntax error on line {line}.")]
pub struct SyntaxError {
    pub path: String,
    pub line: usize,
    #[source_code]
    pub src: NamedSource<Arc<str>>,
    #[label("{detail}")]
    pub span: SourceSpan,
    pub detail: String,
}

/// One thing wrong with a definition, pointing at its line in the file.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("{message}")]
pub struct Problem {
    pub message: String,
    #[source_code]
    pub src: NamedSource<Arc<str>>,
    #[label]
    pub span: SourceSpan,
}
