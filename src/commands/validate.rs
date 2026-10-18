use std::path::PathBuf;
use std::process::ExitCode;

use crate::commands::Verbosity;
use crate::definition;
use crate::error::Error;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// The workflow definition, in YAML or JSON
    file: PathBuf,
}

pub fn validate(args: Args, verbosity: Verbosity) -> Result<ExitCode, Error> {
    definition::load(&args.file)?;
    if verbosity != Verbosity::Quiet {
        println!("{} is valid, and marche can run it.", args.file.display());
    }
    Ok(ExitCode::SUCCESS)
}
