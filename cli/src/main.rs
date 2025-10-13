mod apple;
mod clean;
mod config;
mod create;
mod devices;
mod doctor;
mod package;
mod run;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;
use tracing_subscriber::{FmtSubscriber, filter::LevelFilter, fmt::format::FmtSpan};

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
    /// List available simulators and connected devices
    Devices(devices::DevicesArgs),
    /// Build distributable artifacts without launching them
    Package(package::PackageArgs),
}

fn main() {
    if let Err(err) = run_cli() {
        let icon = style("✖").red();
        eprintln!(
            "{} {}",
            icon,
            style("WaterUI CLI encountered an error").red().bold()
        );

        let error_text = err.to_string();
        let mut lines = error_text.lines().filter(|line| !line.trim().is_empty());
        if let Some(first) = lines.next() {
            eprintln!("  {}", style(first).red());
        }
        for line in lines {
            if line.trim_start().to_ascii_lowercase().starts_with("hint")
                || line.trim_start().starts_with("If ")
            {
                eprintln!(
                    "  {} {}",
                    style("Hint:").yellow().bold(),
                    style(line.trim_start_matches("Hint:").trim()).yellow()
                );
            } else {
                eprintln!("  {}", style(line).dim());
            }
        }

        for cause in err.chain().skip(1) {
            let cause_str = cause.to_string();
            if cause_str.trim().is_empty() {
                continue;
            }
            eprintln!("  {} {}", style("•").dim(), style(cause_str).dim());
        }

        std::process::exit(1);
    }
}

fn run_cli() -> Result<()> {
    let cli = Cli::parse();

    let level = match cli.verbose {
        0 => LevelFilter::INFO,
        1 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_span_events(FmtSpan::NONE)
        .without_time()
        .with_target(false)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    match cli.command {
        Commands::Create(args) => create::run(args),
        Commands::Run(args) => run::run(args),
        Commands::Doctor(args) => doctor::run(args),
        Commands::Clean(args) => clean::run(args),
        Commands::Devices(args) => devices::run(args),
        Commands::Package(args) => package::run(args),
    }
}
