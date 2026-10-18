//! Every error marche prints. The wording is spec: keep it in step with
//! `features/`.

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum Error {
    #[error("Couldn't read {path}: {source}.")]
    Read {
        path: String,
        source: std::io::Error,
    },

    #[error("{path} isn't YAML or JSON: {message}")]
    Parse { path: String, message: String },

    #[error("{path} isn't a valid workflow definition.\n{}", errors.join("\n"))]
    Invalid { path: String, errors: Vec<String> },

    #[error("{path} can't run on marche yet.\n{}", problems.join("\n"))]
    Unsupported { path: String, problems: Vec<String> },

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
