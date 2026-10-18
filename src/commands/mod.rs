//! One module per subcommand.

pub mod cancel;
pub mod guard;
pub mod list;
pub mod run;
pub mod show;
pub mod status;
pub mod validate;
pub mod worker;

/// How much a command prints besides its result, from `-q` and `-v`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    Quiet,
    Normal,
    Verbose,
}
