//! `WaterUI` CLI entry point.

mod commands;
mod project_path;
mod shell;
mod toolchain_checks;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;
use futures::future::{self, Either};
use tracing_subscriber::EnvFilter;

use commands::{
    backend, build, clean, create, device, devices, doctor, inspector, package, preview, run,
};

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

    /// Manage project backends.
    Backend(backend::Args),

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

    /// Manage individual devices.
    Device(device::Args),

    /// List available devices.
    Devices(devices::Args),

    /// Preview a view function as PNG.
    Preview(preview::Args),

    /// Launch the `WaterUI` inspector app.
    Inspector(inspector::Args),
}

fn main() -> Result<()> {
    color_eyre::config::HookBuilder::default()
        .display_location_section(false)
        .display_env_section(false)
        .install()?;

    init_cli_tracing();

    let cli = Cli::parse();

    // Initialize global shell
    shell::init(cli.json);

    // Set up Ctrl+C handler
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let cancelled = Arc::clone(&cancelled);
        ctrlc::set_handler(move || {
            cancelled.store(true, Ordering::SeqCst);
        })
        .expect("failed to set Ctrl+C handler");
    }

    smol::block_on({
        let cancelled = Arc::clone(&cancelled);
        async move {
            waterui_cli::water_dir::ensure_global_config().await?;

            let ctrl_c_future = async {
                // Poll until cancelled
                loop {
                    if cancelled.load(Ordering::SeqCst) {
                        return;
                    }
                    smol::Timer::after(std::time::Duration::from_millis(50)).await;
                }
            };

            let command = async {
                match cli.command {
                    Commands::Create(args) => create::run(args).await,
                    Commands::Backend(args) => backend::run(args).await,
                    Commands::Run(args) => run::run(args).await,
                    Commands::Build(args) => build::run(args).await,

                    Commands::Package(args) => package::run(args).await,
                    Commands::Clean(args) => clean::run(args).await,
                    Commands::Doctor(args) => doctor::run(args).await,
                    Commands::Device(args) => device::run(args).await,
                    Commands::Devices(args) => devices::run(args).await,
                    Commands::Preview(args) => preview::run(args).await,
                    Commands::Inspector(args) => inspector::run(args).await,
                }
            };

            // Race between command execution and Ctrl+C
            let command = std::pin::pin!(command);
            let cancel = std::pin::pin!(ctrl_c_future);

            let result = match future::select(command, cancel).await {
                Either::Left((result, _)) => {
                    // Command completed - check if it failed due to cancellation
                    if cancelled.load(Ordering::SeqCst) {
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
            };

            // Clear progress bars to ensure clean exit
            shell::clear();

            result
        }
    })
}

fn init_cli_tracing() {
    if std::env::var_os("RUST_LOG").is_none() {
        return;
    }

    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}
