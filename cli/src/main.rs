mod config;
mod create;
mod run;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};
use util::LogLevel;

#[derive(Parser)]
#[command(name = "water")]
#[command(about = "CLI of WaterUI", long_about = None)]
#[command(version, author)]
struct Cli {
    /// Increase output verbosity (-v, -vv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactively create a new WaterUI application project
    Create(create::CreateArgs),
    /// Build and run a WaterUI application with SwiftUI hot reload support
    Run(run::RunArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    util::init_logging(LogLevel::from_count(cli.verbose));

    match cli.command {
        Commands::Create(args) => create::run(args),
        Commands::Run(args) => run::run(args),
    }
}
