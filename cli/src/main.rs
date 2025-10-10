mod config;
mod create;
mod clean;
mod doctor;
mod run;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;
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
    /// Check for potential problems with the development environment
    Doctor(doctor::DoctorArgs),
    /// Remove build artifacts and platform caches
    Clean(clean::CleanArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    util::init_logging(LogLevel::from_count(cli.verbose));

    match cli.command {
        Commands::Create(args) => create::run(args),
        Commands::Run(args) => run::run(args),
        Commands::Doctor(args) => {
            let pb = ProgressBar::new_spinner();
            pb.enable_steady_tick(Duration::from_millis(80));
            pb.set_style(
                ProgressStyle::with_template("{spinner:.blue} {msg}")
                    .unwrap()
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            );
            doctor::run(args, pb)
        }
        Commands::Clean(args) => clean::run(args),
    }
}
