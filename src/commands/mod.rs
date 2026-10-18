//! One module per subcommand.

pub mod run;
pub mod show;
pub mod validate;
pub mod worker;

/// How much a command prints besides its result, from `-q` and `-v`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    Quiet,
    Normal,
    Verbose,
}
