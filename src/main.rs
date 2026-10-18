use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::commands::Verbosity;

mod commands;
mod definition;
mod error;
mod expr;
mod interpreter;
mod ows;
mod shell;
mod spans;
mod temporal;
mod workflow;

/// Run Open Workflow Specification definitions as durable workflows on Temporal.
#[derive(Debug, Parser)]
#[command(name = "marche", version, about)]
struct Cli {
    #[command(flatten)]
    target: temporal::Target,
    /// Print only the result
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,
    /// Print each task's output or error as it finishes, and each step the worker runs
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start a run of a workflow definition, and wait for it unless --detach is given
    Run(commands::run::Args),
    /// Show a run's tasks in the order they started
    Show(commands::show::Args),
    /// Check a workflow definition without running it
    Validate(commands::validate::Args),
    /// Run the interpreter and the shell activity until interrupted
    Worker,
    /// Run a shell step's command, and stop it when the worker's pipe closes
    #[command(hide = true)]
    Guard(commands::guard::Args),
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    // Unwrapped, so the JSON pointers in messages stay whole for copying.
    let _ = miette::set_hook(Box::new(|_| {
        Box::new(miette::MietteHandlerOpts::new().wrap_lines(false).build())
    }));
    let verbosity = if cli.quiet {
        Verbosity::Quiet
    } else if cli.verbose {
        Verbosity::Verbose
    } else {
        Verbosity::Normal
    };
    let result = match cli.command {
        Command::Run(args) => commands::run::run(args, &cli.target, verbosity).await,
        Command::Show(args) => commands::show::show(args, &cli.target).await,
        Command::Validate(args) => commands::validate::validate(args, verbosity),
        Command::Worker => commands::worker::worker(&cli.target, verbosity).await,
        Command::Guard(args) => commands::guard::guard(args).await,
    };
    result.unwrap_or_else(|error| {
        let code = error.exit_code();
        eprintln!("{:?}", miette::Report::new(error));
        code
    })
}
