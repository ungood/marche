use cucumber::gherkin::Step;
use cucumber::{then, when};

use super::world::{MarcheWorld, docstring, marche, plain};

#[when(expr = "I run {string}")]
fn i_run(world: &mut MarcheWorld, command: String) {
    let mut args = command.split_whitespace();
    assert_eq!(args.next(), Some("marche"), "commands start with marche");
    let output = std::process::Command::new(marche())
        .args(args)
        .current_dir(world.work_dir())
        .env("NO_COLOR", "1")
        .env("XDG_STATE_HOME", world.state_dir())
        .output()
        .expect("run marche");
    world.last = Some(output);
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

#[then("the output says:")]
fn output_says(world: &mut MarcheWorld, step: &Step) {
    let expected = docstring(step);
    let actual = plain(&world.last().stdout);
    for line in expected
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let line = plain(line.as_bytes());
        assert!(
            actual.contains(&line),
            "output doesn't say {line:?}:\n{actual}"
        );
    }
}
