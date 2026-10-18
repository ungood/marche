use clap::Parser;

/// Run Open Workflow Specification definitions as durable workflows on Temporal.
#[derive(Debug, Parser)]
#[command(name = "marche", version, about)]
struct Cli {}

fn main() {
    Cli::parse();
}
