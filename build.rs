//! Sets `MARCHE_VERSION`, the version `marche --version` prints.
//!
//! Cargo.toml holds a placeholder, and the git tags hold the real versions.
//! A release is built from its tag, such as `v0.2.0`, and prints `0.2.0`.
//! Any other build says how far it is from the last release, such as
//! `0.2.0-3-gabc1234`. Without git history, it prints the placeholder.

use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The version from what `git describe` printed, without the tag's `v`, or
/// the placeholder when there's no release tag to describe.
pub(crate) fn version(described: Option<&str>, placeholder: &str) -> String {
    described
        .and_then(|tag| tag.strip_prefix('v'))
        .unwrap_or(placeholder)
        .to_string()
}

fn main() {
    let version = version(
        git(&["describe", "--tags", "--match", "v[0-9]*"]).as_deref(),
        &std::env::var("CARGO_PKG_VERSION").expect("cargo sets it"),
    );
    println!("cargo:rustc-env=MARCHE_VERSION={version}");

    // Describe again when HEAD moves, a commit lands on its branch, or tags
    // change. A worktree keeps its HEAD apart from the shared refs.
    let watch = [
        git(&["rev-parse", "--git-path", "HEAD"]),
        git(&["rev-parse", "--symbolic-full-name", "HEAD"])
            .and_then(|branch| git(&["rev-parse", "--git-path", &branch])),
        git(&["rev-parse", "--git-path", "refs/tags"]),
        git(&["rev-parse", "--git-path", "packed-refs"]),
    ];
    for path in watch.into_iter().flatten() {
        // Cargo reruns the script every build for a path that doesn't exist.
        if Path::new(&path).exists() {
            println!("cargo:rerun-if-changed={path}");
        }
    }
}
