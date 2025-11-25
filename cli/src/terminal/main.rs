//! `WaterUI` CLI executable.

#![allow(clippy::module_name_repetitions)]

mod command;
mod ui;
mod util;

use clap::{Parser, Subcommand};
use color_eyre::{
    config::HookBuilder,
    eyre::{Report, Result},
};
use command::{
    BackendCommands, BuildCommands, CleanArgs, CleanReport, CleanStatus, CreateArgs, DevicesArgs,
    DoctorArgs, DoctorReport, PackageArgs, RunArgs,
};
use console::style;
use dialoguer::Confirm;
use tracing::{info, warn};
use waterui_cli::output::{self, OutputFormat};

#[derive(Parser, Debug)]
#[command(name = "water", version, about = "WaterUI command line interface")]
struct Cli {
    /// Emit machine-readable JSON output
    #[arg(long)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Scaffold a new `WaterUI` project
    Create(CreateArgs),
    /// Run a `WaterUI` project
    Run(RunArgs),
    /// Build native artifacts without launching them
    Build {
        #[command(subcommand)]
        command: BuildCommands,
    },
    /// Package project artifacts
    Package(PackageArgs),
    /// Manage project backends
    #[command(subcommand)]
    Backend(BackendCommands),
    /// Clean build artifacts and caches
    Clean(CleanArgs),
    /// Diagnose toolchain issues
    Doctor(DoctorArgs),
    /// List available devices and simulators
    Devices(DevicesArgs),
}

fn main() {
    if let Err(error) = run_cli() {
        render_friendly_error(&error);
        std::process::exit(1);
    }
}

fn run_cli() -> Result<()> {
    // Set up color_eyre error reporting with a custom hook
    // Dear Codex, DO NOT REMOVE it: this is needed to get proper error reports
    HookBuilder::default()
        .display_env_section(false)
        .issue_url("https://github.com/water-rs/waterui/issues/new")
        .panic_section("It looks like WaterUI CLI encountered a bug")
        .install()?;

    init_tracing();

    let cli = Cli::parse();
    let format = if cli.json {
        OutputFormat::Json
    } else {
        OutputFormat::Human
    };
    output::set_global_output_format(format);

    match cli.command {
        Commands::Create(args) => {
            let report = command::create::run(args)?;
            emit_or_print(&report, format, |report| {
                info!("Project created at {}", report.project_dir);
                info!("Backends: {}", report.backends.join(", "));
            })?;
        }
        Commands::Run(args) => {
            command::run::run(args)?;
        }
        Commands::Build { command } => {
            let report = command::build::run(command)?;
            emit_or_print(&report, format, |report| {
                info!(
                    "Built {} artifact(s) for {} ({})",
                    report.artifacts.len(),
                    report.platform,
                    report.profile
                );
            })?;
        }
        Commands::Package(args) => {
            let report = command::package::run(args)?;
            emit_or_print(&report, format, |report| {
                if report.artifacts.is_empty() {
                    info!("No artifacts produced.");
                } else {
                    info!("Artifacts:");
                    for artifact in &report.artifacts {
                        info!("  {} -> {}", artifact.platform, artifact.path);
                    }
                }
            })?;
        }
        Commands::Backend(subcommand) => match subcommand {
            BackendCommands::Add(args) => {
                let report = command::add_backend::run(args)?;
                emit_or_print(&report, format, |report| {
                    info!(
                        "Backend {} added. Updated config at {}",
                        report.backend, report.config_path
                    );
                })?;
            }
            BackendCommands::Update(args) => {
                let report = command::backend::update(args)?;
                emit_or_print(&report, format, |report| {
                    info!(
                        "Backend {} update status: {:?}",
                        report.backend, report.status
                    );
                    if let Some(from) = &report.from_version {
                        if let Some(to) = &report.to_version {
                            info!("Version: {from} -> {to}");
                        }
                    }
                    if let Some(message) = &report.message {
                        info!("{message}");
                    }
                })?;
            }
            BackendCommands::Upgrade(args) => {
                let report = command::backend::upgrade(args)?;
                emit_or_print(&report, format, |report| {
                    info!(
                        "Backend {} upgrade status: {:?}",
                        report.backend, report.status
                    );
                    if let Some(from) = &report.from_version {
                        if let Some(to) = &report.to_version {
                            info!("Version: {from} -> {to}");
                        }
                    }
                    if let Some(message) = &report.message {
                        info!("{message}");
                    }
                })?;
            }
            BackendCommands::List(args) => {
                let report = command::backend::list(args)?;
                emit_or_print(&report, format, |report| {
                    info!("Configured backends: {}", report.entries.len());
                })?;
            }
        },
        Commands::Clean(mut args) => execute_clean(&mut args, format)?,
        Commands::Doctor(args) => {
            let report = command::doctor::run(args)?;
            emit_or_print(&report, format, render_doctor_report)?;
        }
        Commands::Devices(args) => {
            let devices = command::devices::run(args)?;
            if format.is_json() {
                output::emit_json(&devices)?;
            } else if devices.is_empty() {
                warn!("No devices detected. Connect a device or start a simulator.");
            } else {
                render_device_table(&devices);
            }
        }
    }

    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt::Subscriber::builder()
        .with_env_filter(filter)
        .without_time()
        .with_target(false)
        .try_init();
}

fn emit_or_print<T, F>(value: &T, format: OutputFormat, printer: F) -> Result<()>
where
    T: serde::Serialize,
    F: FnOnce(&T),
{
    if format.is_json() {
        output::emit_json(value)
    } else {
        printer(value);
        Ok(())
    }
}

fn render_friendly_error(error: &Report) {
    let messages = collect_error_messages(error);
    let mut suggestions = derive_suggestions(&messages);

    if output::global_output_format().is_json() {
        if let Some(message) = messages.first() {
            eprintln!("Error: {message}");
        }
        for suggestion in suggestions.drain(..) {
            eprintln!("Hint: {suggestion}");
        }
        return;
    }

    if let Some(message) = messages.first() {
        eprintln!("{} {}", style("Error:").red().bold(), style(message).red());
    } else {
        eprintln!(
            "{} {}",
            style("Error:").red().bold(),
            style("unexpected failure").red()
        );
    }

    if messages.len() > 1 {
        eprintln!(
            "{} {}",
            style("Caused by:").dim(),
            style(&messages[1]).dim()
        );
        for message in messages.iter().skip(2) {
            eprintln!("    - {}", style(message).dim());
        }
    }

    if let Some(hint) = suggestions.first() {
        eprintln!(
            "{} {}",
            style("Hint:").yellow().bold(),
            style(hint).yellow()
        );
        for hint in suggestions.iter().skip(1) {
            eprintln!("    - {}", style(hint).yellow());
        }
    }
}

fn collect_error_messages(error: &Report) -> Vec<String> {
    error
        .chain()
        .map(|cause| cause.to_string())
        .map(|message| {
            message
                .trim()
                .trim_start_matches("Error:")
                .trim()
                .to_string()
        })
        .filter(|message| !message.is_empty())
        .fold(Vec::new(), |mut messages, message| {
            if messages
                .last()
                .map_or(true, |previous| previous != &message)
            {
                messages.push(message);
            }
            messages
        })
}

fn derive_suggestions(messages: &[String]) -> Vec<String> {
    let mut suggestions = Vec::new();
    let lowered: Vec<String> = messages.iter().map(|msg| msg.to_lowercase()).collect();

    if lowered
        .iter()
        .any(|msg| msg.contains("toolchain requirement"))
    {
        suggestions.push("Run `water doctor` to diagnose and fix toolchain issues.".to_string());
    }

    if lowered.iter().any(|msg| msg.contains("not found")) {
        suggestions
            .push("Install the missing tool or ensure it is available on your PATH.".to_string());
    }

    if suggestions.is_empty() {
        suggestions.push(
            "Re-run with `-vv` for verbose logs and retry after addressing the error above."
                .to_string(),
        );
    }

    suggestions.dedup();
    suggestions
}

fn execute_clean(args: &mut CleanArgs, format: OutputFormat) -> Result<()> {
    loop {
        let report = command::clean::run(args.clone());
        match report.status {
            CleanStatus::PendingConfirmation if !format.is_json() => {
                render_pending_actions(&report);
                let proceed = Confirm::new()
                    .with_prompt("Continue with cleanup?")
                    .default(false)
                    .interact()?;
                if proceed {
                    args.yes = true;
                    continue;
                }
                warn!("Cleanup aborted.");
                return Ok(());
            }
            _ => {
                emit_or_print(&report, format, render_clean_report)?;
                return Ok(());
            }
        }
    }
}

fn render_pending_actions(report: &CleanReport) {
    println!(
        "The following cleanup actions will be performed in {}:",
        report.workspace
    );
    for action in &report.actions {
        println!("  - {}", action.description);
    }
}

fn render_clean_report(report: &CleanReport) {
    println!("Cleanup status: {:?}", report.status);
    for action in &report.actions {
        let detail = action
            .detail
            .as_deref()
            .map(|d| format!(" ({d})"))
            .unwrap_or_default();
        println!("  - {:?}: {}{}", action.result, action.description, detail);
        if let Some(error) = &action.error {
            println!("      error: {error}");
        }
    }
    if report.status == CleanStatus::Error {
        for error in &report.errors {
            println!("Error: {error}");
        }
    }
}

fn render_doctor_report(report: &DoctorReport) {
    println!("Doctor status: {:?}", report.status);
    for section in &report.sections {
        for (idx, line) in section.render().iter().enumerate() {
            if idx == 0 {
                println!();
            }
            println!("{line}");
        }
    }
    if let Some(fixes) = &report.applied_fixes {
        if !fixes.is_empty() {
            println!();
            println!("Applied fixes:");
            for fix in fixes {
                println!("  - {} => {:?}", fix.description, fix.outcome);
            }
        }
    }
}

fn render_device_table(devices: &[waterui_cli::device::DeviceInfo]) {
    use std::collections::BTreeMap;
    use waterui_cli::device::DeviceKind;

    let mut grouped: BTreeMap<&str, Vec<&waterui_cli::device::DeviceInfo>> = BTreeMap::new();
    for device in devices {
        grouped.entry(&device.platform).or_default().push(device);
    }

    for (idx, (platform, list)) in grouped.iter().enumerate() {
        if idx > 0 {
            println!();
        }
        println!("{platform}");

        let mut items = list.clone();
        items.sort_by(|a, b| {
            let rank = |kind: &DeviceKind| match kind {
                DeviceKind::Device => 0,
                DeviceKind::Simulator => 1,
                DeviceKind::Emulator => 2,
            };
            rank(&a.kind)
                .cmp(&rank(&b.kind))
                .then_with(|| a.name.cmp(&b.name))
        });

        for device in items {
            let state = device.state.as_deref().unwrap_or_else(|| {
                if device.kind == DeviceKind::Emulator {
                    "stopped"
                } else {
                    "-"
                }
            });
            println!("  • {} ({:?}, {})", device.name, device.kind, state);
            println!("      id: {}", device.identifier);
            if let Some(detail) = &device.detail {
                println!("      {detail}");
            }
        }
    }
}
