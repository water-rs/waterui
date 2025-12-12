//! `WaterUI` CLI entry point.

mod commands;
mod shell;

use std::sync::atomic::{AtomicBool, Ordering};

use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;
use futures::future::{self, Either};

use commands::{build, clean, create, devices, doctor, package, run};

/// Flag to track if Ctrl+C was pressed.
static CANCELLED: AtomicBool = AtomicBool::new(false);

/// Mark the CLI as cancelled (called from Ctrl+C handler).
fn set_cancelled() {
    CANCELLED.store(true, Ordering::SeqCst);
}

/// Check if the CLI was cancelled by Ctrl+C.
fn is_cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

/// `WaterUI` command line interface.
#[derive(Parser, Debug)]
#[command(name = "water", version, about, long_about = None)]
struct Cli {
    /// Output in JSON format (machine-readable).
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create a new `WaterUI` project.
    Create(create::Args),

    /// Build and run on device/simulator.
    Run(run::Args),

    /// Build the project for a platform.
    Build(build::Args),

    /// Package for distribution.
    Package(package::Args),

    /// Clean build artifacts.
    Clean(clean::Args),

    /// Check development environment.
    Doctor(doctor::Args),

    /// List available devices.
    Devices(devices::Args),
}

fn main() -> Result<()> {
    color_eyre::config::HookBuilder::default()
        .display_location_section(false)
        .display_env_section(false)
        .install()?;

    let cli = Cli::parse();

    // Initialize global shell
    shell::init(cli.json);

    // Set up Ctrl+C handler
    ctrlc::set_handler(set_cancelled).expect("failed to set Ctrl+C handler");

    smol::block_on(async {
        let ctrl_c_future = async {
            // Poll until cancelled
            loop {
                if is_cancelled() {
                    return;
                }
                smol::Timer::after(std::time::Duration::from_millis(50)).await;
            }
        };

        let command = async {
            match cli.command {
                Commands::Create(args) => create::run(args).await,
                Commands::Run(args) => run::run(args).await,
                Commands::Build(args) => build::run(args).await,
                Commands::Package(args) => package::run(args).await,
                Commands::Clean(args) => clean::run(args).await,
                Commands::Doctor(args) => doctor::run(args).await,
                Commands::Devices(args) => devices::run(args).await,
            }
        };

        // Race between command execution and Ctrl+C
        let command = std::pin::pin!(command);
        let cancel = std::pin::pin!(ctrl_c_future);

        match future::select(command, cancel).await {
            Either::Left((result, _)) => {
                // Command completed - check if it failed due to cancellation
                if is_cancelled() {
                    // Suppress errors caused by Ctrl+C interruption
                    Ok(())
                } else {
                    result
                }
            }
            Either::Right(((), _)) => {
                // Ctrl+C pressed - exit gracefully
                // The command future is dropped here, triggering cleanup
                Ok(())
            }
        }
    })
}
