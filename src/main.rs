use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod commands;
mod definition;
mod error;
mod expr;
mod interpreter;
mod ows;
mod shell;
mod temporal;
mod workflow;

/// Run Open Workflow Specification definitions as durable workflows on Temporal.
#[derive(Debug, Parser)]
#[command(name = "marche", version, about)]
struct Cli {
    #[command(flatten)]
    target: temporal::Target,
    /// Print only the result
    #[arg(short, long, global = true)]
    quiet: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start a run of a workflow definition, and wait for it unless --detach is given
    Run(commands::run::Args),
    /// Show a run's tasks in the order they started
    Show(commands::show::Args),
    /// Run the interpreter and the shell activity until interrupted
    Worker,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Run(args) => commands::run::run(args, &cli.target, cli.quiet).await,
        Command::Show(args) => commands::show::show(args, &cli.target).await,
        Command::Worker => commands::worker::worker(&cli.target).await,
    };
    result.unwrap_or_else(|error| {
        eprintln!("{:?}", miette::Report::new(error));
        ExitCode::FAILURE
    })
}
