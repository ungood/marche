use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use serde_json::Value;

use super::ows::contains;
use super::world::{MarcheWorld, docstring, marche, plain};

/// Subcommands that talk to Temporal, so `I run` points them at the test server.
const TEMPORAL_COMMANDS: &[&str] = &["run", "show", "status", "list", "cancel"];

#[given(regex = r#"^the file "([^"]+)":$"#)]
fn file(world: &mut MarcheWorld, path: String, step: &Step) {
    world.write(&path, &format!("{}\n", docstring(step)));
}

#[when(regex = r#"^I run "(.+)"$"#)]
fn i_run(world: &mut MarcheWorld, command: String) {
    let words = shell_words::split(&command).expect("the command splits into words");
    let (program, args) = words.split_first().expect("the command isn't empty");
    assert_eq!(program, "marche", "commands start with marche");
    let mut run = std::process::Command::new(marche());
    run.args(args)
        .current_dir(world.work_dir())
        .env("NO_COLOR", "1")
        .env("XDG_STATE_HOME", world.state_dir());
    if args
        .first()
        .is_some_and(|command| TEMPORAL_COMMANDS.contains(&command.as_str()))
    {
        run.env("TEMPORAL_ADDRESS", world.address());
    }
    world.last = Some(run.output().expect("run marche"));
}

#[then("the command succeeds")]
fn succeeds(world: &mut MarcheWorld) {
    let output = world.last();
    assert!(
        output.status.success(),
        "marche failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[then("the command fails")]
fn fails(world: &mut MarcheWorld) {
    assert!(!world.last().status.success(), "marche succeeded");
}

/// Checks that every line of the doc string appears in the text.
fn says(actual: &[u8], step: &Step, what: &str) {
    let actual = plain(actual);
    for line in docstring(step)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let line = plain(line.as_bytes());
        assert!(
            actual.contains(&line),
            "{what} doesn't say {line:?}:\n{actual}"
        );
    }
}

#[then("the output says:")]
fn output_says(world: &mut MarcheWorld, step: &Step) {
    says(&world.last().stdout, step, "the output");
}

#[then("the error says:")]
fn error_says(world: &mut MarcheWorld, step: &Step) {
    says(&world.last().stderr, step, "the error");
}

#[then("marche prints:")]
fn prints(world: &mut MarcheWorld, step: &Step) {
    let output = world.last();
    let both = [output.stderr.as_slice(), output.stdout.as_slice()].concat();
    says(&both, step, "marche");
}

#[then("the JSON output contains:")]
fn json_contains(world: &mut MarcheWorld, step: &Step) {
    let actual: Value = serde_json::from_slice(&world.last().stdout).expect("the output is JSON");
    let expected: Value = serde_json::from_str(&docstring(step)).expect("the doc string is JSON");
    assert!(
        contains(&actual, &expected),
        "{actual:#} doesn't contain {expected:#}"
    );
}

#[then(regex = r#"^the process whose ID is in "([^"]+)" has exited$"#)]
fn process_exited(world: &mut MarcheWorld, path: String) {
    let text = std::fs::read_to_string(world.work_dir().join(&path)).expect("read the PID file");
    let pid: libc::pid_t = text.trim().parse().expect("the file holds a process ID");
    // SAFETY: signal 0 sends nothing; it only checks that the process exists.
    let running = unsafe { libc::kill(pid, 0) } == 0;
    assert!(!running, "process {pid} from {path} is still running");
}
