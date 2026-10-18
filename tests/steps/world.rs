use std::path::{Path, PathBuf};
use std::process::{Child, Output};

use cucumber::World;
use serde_json::Value;
use tempfile::TempDir;

use super::services::Services;

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct MarcheWorld {
    root: TempDir,
    /// The result of the last `I run`.
    pub last: Option<Output>,
    /// The workflow definition from `a workflow with definition:`.
    pub definition: Option<String>,
    /// The input from `the workflow input is:`.
    pub input: Option<Value>,
    /// `marche run --json` output from `the workflow is executed`.
    pub run: Option<Value>,
    /// `marche show --json` output for the same run.
    pub show: Option<Value>,
    /// A server and worker of the scenario's own, for scenarios that kill the worker.
    pub services: Option<Services>,
    /// A `marche run --json` still in progress.
    pub pending: Option<Child>,
    /// Environment variables from `the environment variable ... is ...`.
    pub env: Vec<(String, String)>,
    /// The ID of the run started last by `has run` or `is running`.
    pub run_id: Option<String>,
    /// How long `the workflow is executed` took, or how long the run took to
    /// stop after `the workflow is cancelled`.
    pub took: Option<std::time::Duration>,
}

impl MarcheWorld {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("create temp dir");
        for dir in ["work", "state"] {
            std::fs::create_dir_all(root.path().join(dir)).expect("create test dir");
        }
        Self {
            root,
            last: None,
            definition: None,
            input: None,
            run: None,
            show: None,
            services: None,
            pending: None,
            env: Vec::new(),
            run_id: None,
            took: None,
        }
    }

    /// Kills a run still in progress, and stops the scenario's own server and worker.
    pub fn stop(&mut self) {
        if let Some(mut run) = self.pending.take() {
            let _ = run.kill();
            let _ = run.wait();
        }
        self.services = None;
    }

    /// The directory commands run in.
    pub fn work_dir(&self) -> PathBuf {
        self.root.path().join("work")
    }

    /// Temporal's configuration file, so tests never read the real one.
    pub fn temporal_config(&self) -> PathBuf {
        self.root.path().join("temporal.toml")
    }

    /// Where marche keeps run logs, so tests never write to the real home directory.
    pub fn state_dir(&self) -> PathBuf {
        self.root.path().join("state")
    }

    /// The address of the scenario's own server, or else the shared one.
    pub fn address(&self) -> String {
        match &self.services {
            Some(services) => services.address.clone(),
            None => super::services::address(),
        }
    }

    /// `text` with `$RUN` and `$ULID` replaced by the ID of the scenario's run
    /// and its ULID.
    pub fn expand(&self, text: &str) -> String {
        let Some(run) = &self.run_id else {
            return text.to_string();
        };
        let ulid = run.rsplit('-').next().unwrap_or(run);
        text.replace("$RUN", run).replace("$ULID", ulid)
    }

    pub fn last(&self) -> &Output {
        self.last.as_ref().expect("no command has run")
    }

    pub fn write(&self, name: &str, content: &str) -> PathBuf {
        let path = self.work_dir().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create test directory");
        }
        std::fs::write(&path, content).expect("write test file");
        path
    }
}

/// The marche binary under test.
pub fn marche() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_marche"))
}

/// Text with colors removed and runs of whitespace collapsed, for matching messages.
/// miette's decoration, such as the bar it draws beside a wrapped message, is
/// removed too.
pub fn plain(bytes: &[u8]) -> String {
    let stripped = strip_ansi_escapes::strip(bytes);
    String::from_utf8_lossy(&stripped)
        .replace(|c| "│×╭╰─┬┴├┤╮╯·▲⚠✖‽".contains(c), " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A step's doc string. The gherkin parser keeps a media type such as `yaml`
/// on the first line, and then can't remove the indentation, so this does both.
pub fn docstring(step: &cucumber::gherkin::Step) -> String {
    let raw = step
        .docstring
        .as_deref()
        .expect("the step has a doc string");
    let body = raw.split_once('\n').map_or("", |(_, body)| body);
    let indent = body
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    body.lines()
        .map(|line| line.get(indent..).unwrap_or("").trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}
