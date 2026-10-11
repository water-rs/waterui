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

impl Commands {
    /// The project directory a command works on — the one whose managed
    /// build cache the background sweep keeps — or `None` for a command
    /// that opens no project.
    fn project_dir(&self) -> Option<&std::path::Path> {
        match self {
            Self::Run(args) => Some(args.project_dir()),
            Self::Bench(args) => Some(args.project_dir()),
            Self::Build(args) => Some(args.project_dir()),
            Self::Package(args) => Some(args.project_dir()),
            Self::Clean(args) => Some(args.project_dir()),
            Self::Fetch(args) => Some(args.project_dir()),
            Self::Preview(args) => Some(args.project_dir()),
            Self::Inspector(args) => Some(args.project_dir()),
            Self::Mcp(args) => Some(args.project_dir()),
            Self::Channel(args) => Some(args.project_dir()),
            Self::Create(_)
            | Self::Init(_)
            | Self::Doctor(_)
            | Self::Device(_)
            | Self::Devices(_)
            | Self::Gc(_)
            | Self::Update(_)
            | Self::Completions(_) => None,
        }
    }
}

/// Sweep stale managed build caches in the background, keeping the cache of
/// the project `command` works on. Only this binary requests the sweep: it
/// re-launches the running executable, which is `water` only here.
async fn request_build_cache_cleanup(command: &Commands) {
    let Some(project_dir) = command.project_dir() else {
        return;
    };
    // A path that does not resolve names no project, so no sweep starts;
    // the command itself then fails with the canonicalization error.
    let Ok(project_root) = smol::fs::canonicalize(project_dir).await else {
        return;
    };
    if let Err(error) = waterui_cli::water_dir::spawn_build_cache_cleanup(
        &waterui_cli::toolchain::Host::current(),
        &project_root,
    ) {
        tracing::warn!(
            project_root = %project_root.display(),
            "Failed to start the build-cache cleanup: {error:#}"
        );
    }
}

fn main() -> Result<()> {
    color_eyre::config::HookBuilder::default()
        .display_location_section(false)
        .display_env_section(false)
        .install()?;

    init_cli_tracing(&waterui_cli::toolchain::Host::current());

    let cli = Cli::parse();

    let shell = shell::Shell::new(cli.json);

    // Interrupts travel a channel, not a flag: the `ctrlc` handler fires on
    // Ctrl+C, and on SIGTERM/SIGHUP through the same path (the `termination`
    // feature of `ctrlc`). `run` receives the channel and owns its own
    // shutdown — it must keep running after the first interrupt so its stop
    // sequence and the app's shutdown output can complete — while every
    // other command is raced against it and dropped.
    let (interrupt_tx, interrupt_rx) = async_channel::unbounded();
    ctrlc::set_handler(move || {
        let _ = interrupt_tx.try_send(());
    })
    .expect("failed to set Ctrl+C handler");

    smol::block_on(async move {
        waterui_cli::water_dir::ensure_global_config(&waterui_cli::toolchain::Host::current())
            .await?;
        request_build_cache_cleanup(&cli.command).await;

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

        let result = match cli.command {
            Commands::Run(args) => Box::pin(run::run(&shell, args, interrupt_rx))
                .await
                .map(Some),
            Commands::Create(args) => {
                Box::pin(until_interrupt(create::run(&shell, args), &interrupt_rx)).await
            }
            Commands::Init(args) => {
                Box::pin(until_interrupt(init::run(&shell, args), &interrupt_rx)).await
            }
            Commands::Channel(args) => {
                Box::pin(until_interrupt(channel::run(&shell, args), &interrupt_rx)).await
            }
            Commands::Bench(args) => until_interrupt(bench::run(&shell, args), &interrupt_rx).await,
            Commands::Build(args) => {
                until_interrupt(Box::pin(build::run(&shell, args)), &interrupt_rx).await
            }
            Commands::Package(args) => {
                until_interrupt(Box::pin(package::run(&shell, args)), &interrupt_rx).await
            }
            Commands::Clean(args) => {
                Box::pin(until_interrupt(clean::run(&shell, args), &interrupt_rx)).await
            }
            Commands::Doctor(args) => {
                until_interrupt(doctor::run(&shell, args), &interrupt_rx).await
            }
            Commands::Device(args) => {
                until_interrupt(device::run(&shell, args), &interrupt_rx).await
            }
            Commands::Devices(args) => {
                until_interrupt(devices::run(&shell, args), &interrupt_rx).await
            }
            Commands::Gc(args) => until_interrupt(gc::run(&shell, args), &interrupt_rx).await,
            Commands::Fetch(args) => {
                Box::pin(until_interrupt(fetch::run(&shell, args), &interrupt_rx)).await
            }
            Commands::Preview(args) => {
                until_interrupt(Box::pin(preview::run(&shell, args)), &interrupt_rx).await
            }
            Commands::Inspector(args) => {
                Box::pin(until_interrupt(inspector::run(&shell, args), &interrupt_rx)).await
            }
            Commands::Mcp(args) => until_interrupt(mcp::run(&shell, args), &interrupt_rx).await,
            Commands::Update(args) => {
                until_interrupt(update::run(&shell, args), &interrupt_rx).await
            }
            Commands::Completions(args) => {
                until_interrupt(async { completions::run(&args) }, &interrupt_rx).await
            }
        }
        .map(|_: Option<()>| ());

        // Clear progress bars to ensure clean exit
        shell.clear();

        if result.is_ok()
            && off_update_hot_path
            && let Some(notice) = waterui_cli::self_update::passive_update_notice(
                &waterui_cli::toolchain::Host::current(),
            )
            .await
        {
            crate::note!(shell, "{notice}");
        }

        result
    })
}

/// Race `command` against the interrupt channel; an interrupt ends it with
/// `None`. An error the command returns while another interrupt is already
/// queued is the interrupt's consequence, so it ends with `None` as well.
pub(crate) async fn until_interrupt<T>(
    command: impl std::future::Future<Output = Result<T>>,
    interrupts: &smol::channel::Receiver<()>,
) -> Result<Option<T>> {
    let command = std::pin::pin!(command);
    let interrupt = std::pin::pin!(interrupts.recv());
    match future::select(command, interrupt).await {
        Either::Left((Err(_), _)) if interrupts.try_recv().is_ok() => Ok(None),
        Either::Left((result, _)) => result.map(Some),
        Either::Right(_) => Ok(None),
    }
}

/// Install the stderr subscriber when `host` sets `RUST_LOG`, filtered by
/// its directives.
fn init_cli_tracing(host: &waterui_cli::toolchain::Host) {
    let Some(directives) = host.env("RUST_LOG") else {
        return;
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::builder().parse_lossy(directives.to_string_lossy()))
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}
