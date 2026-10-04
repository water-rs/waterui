//! `WaterUI` CLI entry point.

mod commands;
/// The library's view of the pinned framework revision, compiled into this
/// binary's tests too: the `create --template web` tests scaffold against a
/// clone of it.
#[cfg(test)]
#[path = "../pinned_framework/clone.rs"]
mod pinned_framework;
mod project_path;
mod shell;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use clap::{Parser, Subcommand};
use eyre::Result;
use futures_util::future::{self, Either};
use tracing_subscriber::EnvFilter;

use commands::{
    bench, build, channel, clean, completions, create, device, devices, doctor, fetch, gc, init,
    inspector, mcp, package, preview, run, update,
};

/// `WaterUI` command line interface.
#[derive(Parser, Debug)]
#[command(name = "water", version, about, long_about = None)]
pub(crate) struct Cli {
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

    /// Initialize the current directory as a `WaterUI` project, adopting or
    /// scaffolding a web frontend.
    Init(init::Args),

    /// Inspect or explicitly update the project's framework channel.
    Channel(channel::Args),

    /// Build and run on device/simulator.
    Run(run::Args),

    /// Run `#[waterui::bench]` GPU frame benchmarks and collect reports.
    Bench(bench::Args),

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

    /// Garbage-collect stale managed caches.
    Gc(gc::Args),

    /// Download the project's declared fonts into the font cache.
    Fetch(fetch::Args),

    /// Preview a view function as PNG.
    Preview(preview::Args),

    /// Launch the `WaterUI` inspector app.
    Inspector(inspector::Args),

    /// Serve the app to an agent over MCP.
    Mcp(mcp::Args),

    /// Update the `water` CLI itself; `--check` reports without installing.
    Update(update::Args),

    /// Print the shell completion script for a shell.
    Completions(completions::Args),
}

fn main() -> Result<()> {
    // A `RUSTC_WRAPPER` build-std invocation enters the process before any CLI
    // parsing: Cargo calls this binary as `water <rustc> <args…>`.
    if let Some(code) = waterui_cli::rustc_wrapper::wrapper_main() {
        std::process::exit(code);
    }

    color_eyre::config::HookBuilder::default()
        .display_location_section(false)
        .display_env_section(false)
        .install()?;

    init_cli_tracing();

    let cli = Cli::parse();

    let shell = shell::Shell::new(cli.json);

    // Cancel on Ctrl+C, and on SIGTERM/SIGHUP through the same path (the
    // `termination` feature of `ctrlc`): a plain `kill` then still drops the
    // running command, which is what stops the app and its log stream.
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

            // The passive update check stays off the `build`/`run` hot path,
            // off machine-consumed output (`mcp`, `completions`), and never
            // runs inside `update` itself, which checks explicitly.
            let off_update_hot_path = !matches!(
                cli.command,
                Commands::Build(_)
                    | Commands::Run(_)
                    | Commands::Update(_)
                    | Commands::Mcp(_)
                    | Commands::Completions(_)
            );

            let command = async {
                match cli.command {
                    Commands::Create(args) => create::run(&shell, args).await,
                    Commands::Init(args) => init::run(&shell, args).await,
                    Commands::Channel(args) => channel::run(&shell, args).await,
                    Commands::Run(args) => Box::pin(run::run(&shell, args)).await,
                    Commands::Bench(args) => bench::run(&shell, args).await,
                    Commands::Build(args) => Box::pin(build::run(&shell, args)).await,

                    Commands::Package(args) => Box::pin(package::run(&shell, args)).await,
                    Commands::Clean(args) => clean::run(&shell, args).await,
                    Commands::Doctor(args) => doctor::run(&shell, args).await,
                    Commands::Device(args) => device::run(&shell, args).await,
                    Commands::Devices(args) => devices::run(&shell, args).await,
                    Commands::Gc(args) => gc::run(&shell, args).await,
                    Commands::Fetch(args) => fetch::run(&shell, args).await,
                    Commands::Preview(args) => Box::pin(preview::run(&shell, args)).await,
                    Commands::Inspector(args) => inspector::run(&shell, args).await,
                    Commands::Mcp(args) => mcp::run(&shell, args).await,
                    Commands::Update(args) => update::run(&shell, args).await,
                    Commands::Completions(args) => completions::run(&args),
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
            shell.clear();

            if result.is_ok()
                && off_update_hot_path
                && let Some(notice) = waterui_cli::self_update::passive_update_notice().await
            {
                crate::note!(shell, "{notice}");
            }

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
