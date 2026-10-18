//! The version build.rs takes from `git describe`, for the tags the features
//! can't build from.

#[allow(dead_code)]
#[path = "../build.rs"]
mod build;

const PLACEHOLDER: &str = "0.0.0-development";

#[test]
fn a_release_tag_is_its_version() {
    assert_eq!(build::version(Some("v0.2.0"), PLACEHOLDER), "0.2.0");
}

#[test]
fn a_later_commit_says_how_far_it_is_from_the_release() {
    assert_eq!(
        build::version(Some("v0.2.0-3-gabc1234"), PLACEHOLDER),
        "0.2.0-3-gabc1234"
    );
}

#[test]
fn without_a_release_tag_the_version_is_the_placeholder() {
    assert_eq!(build::version(None, PLACEHOLDER), PLACEHOLDER);
}

#[test]
fn a_tag_without_a_v_is_not_a_release() {
    assert_eq!(build::version(Some("0.2.0"), PLACEHOLDER), PLACEHOLDER);
}
