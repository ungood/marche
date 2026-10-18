use std::path::{Path, PathBuf};
use std::process::Output;

use cucumber::World;
use serde_json::Value;
use tempfile::TempDir;

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
        }
    }

    /// The directory commands run in.
    pub fn work_dir(&self) -> PathBuf {
        self.root.path().join("work")
    }

    /// Where marche keeps run logs, so tests never write to the real home directory.
    pub fn state_dir(&self) -> PathBuf {
        self.root.path().join("state")
    }

    pub fn last(&self) -> &Output {
        self.last.as_ref().expect("no command has run")
    }

    pub fn write(&self, name: &str, content: &str) -> PathBuf {
        let path = self.work_dir().join(name);
        std::fs::write(&path, content).expect("write test file");
        path
    }
}

/// The marche binary under test.
pub fn marche() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_marche"))
}

/// Text with colors removed and runs of whitespace collapsed, for matching messages.
pub fn plain(bytes: &[u8]) -> String {
    let stripped = strip_ansi_escapes::strip(bytes);
    String::from_utf8_lossy(&stripped)
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
