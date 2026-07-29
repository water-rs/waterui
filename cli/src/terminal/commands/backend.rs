//! `water backend` command implementation.

use std::path::Path;

use clap::{Args as ClapArgs, Subcommand, ValueEnum};
use color_eyre::eyre::{Result, bail};
use dialoguer::{Confirm, theme::ColorfulTheme};

use crate::shell::Shell;
use crate::{header, line, note, success, warn};
use waterui_cli::project::{PackageType, Project};

/// Backend management command arguments.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Backend action.
    #[command(subcommand)]
    command: BackendCommand,
}

#[derive(Subcommand, Debug)]
enum BackendCommand {
    /// Add and scaffold a backend.
    Add(AddArgs),
    /// Remove backend configuration and generated files.
    Remove(RemoveArgs),
    /// Show configured backends.
    List,
}

#[derive(ClapArgs, Debug)]
struct AddArgs {
    /// Backend to add.
    backend: BackendName,
}

#[derive(ClapArgs, Debug)]
struct RemoveArgs {
    /// Backend to remove.
    backend: BackendName,
    /// Skip confirmation prompt.
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BackendName {
    Apple,
    Android,
    Gtk4,
    Hydrolysis,
    Esp32,
}

/// Run the backend command.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    let project_path = crate::project_path::canonicalize(Path::new("."))?;
    let mut project = Project::open(&project_path).await?;

    if project.package_type() != PackageType::App {
        bail!(
            "`water backend` is only available for app mode projects.\n\
             Playground backends are fully managed under `~/.water/build_cache/...`."
        );
    }

    match args.command {
        BackendCommand::Add(add) => add_backend(shell, &mut project, add.backend).await,
        BackendCommand::Remove(remove) => {
            remove_backend(shell, &mut project, remove.backend, remove.yes).await
        }
        BackendCommand::List => {
            list_backends(shell, &project);
            Ok(())
        }
    }
}

async fn add_backend(shell: &Shell, project: &mut Project, backend: BackendName) -> Result<()> {
    header!(shell, "Adding backend: {}", backend_name(backend));
    validate_backend_add_on_host(backend)?;

    match backend {
        BackendName::Apple => {
            if project.apple_backend().is_some() {
                note!(shell, "Apple backend already configured");
                return Ok(());
            }
            let spinner = shell.spinner("Scaffolding Apple backend...");
            project.init_apple_backend().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!(shell, "Added Apple backend");
        }
        BackendName::Android => {
            if project.android_backend().is_some() {
                note!(shell, "Android backend already configured");
                return Ok(());
            }
            let spinner = shell.spinner("Scaffolding Android backend...");
            project.init_android_backend().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!(shell, "Added Android backend");
        }
        BackendName::Gtk4 => {
            if project.gtk4_backend().is_some() {
                note!(shell, "GTK4 backend already configured");
                return Ok(());
            }
            let spinner = shell.spinner("Scaffolding GTK4 backend...");
            project.init_gtk4_backend().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!(shell, "Added GTK4 backend");
        }
        BackendName::Hydrolysis => {
            if project.hydrolysis_backend().is_some() {
                note!(shell, "Hydrolysis backend already configured");
                return Ok(());
            }
            let spinner = shell.spinner("Scaffolding hydrolysis backend...");
            project.init_hydrolysis_backend().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!(shell, "Added hydrolysis backend");
        }
        BackendName::Esp32 => {
            if project.esp32_backend().is_some() {
                note!(shell, "ESP32 backend already configured");
                return Ok(());
            }
            let spinner = shell.spinner("Scaffolding ESP32 backend...");
            project.init_esp32_backend().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!(shell, "Added ESP32 backend");
        }
    }

    Ok(())
}

fn validate_backend_add_on_host(backend: BackendName) -> Result<()> {
    match backend {
        BackendName::Gtk4 => {
            if !cfg!(target_os = "linux") {
                bail!("GTK4 backend is only supported on Linux hosts");
            }
        }
        BackendName::Hydrolysis => {
            if !cfg!(any(
                target_os = "macos",
                target_os = "linux",
                target_os = "windows"
            )) {
                bail!("Hydrolysis backend is only supported on macOS, Linux, or Windows hosts");
            }
        }
        // The ESP32 firmware cross-compiles from any host with espup installed.
        BackendName::Apple | BackendName::Android | BackendName::Esp32 => {}
    }

    Ok(())
}

async fn remove_backend(
    shell: &Shell,
    project: &mut Project,
    backend: BackendName,
    yes: bool,
) -> Result<()> {
    if !is_backend_configured(project, backend) {
        bail!("Backend {} is not configured", backend_name(backend));
    }

    if !yes && !shell.is_interactive() {
        bail!("`water backend remove` requires --yes in non-interactive environments");
    }

    if !yes && shell.is_interactive() {
        let confirmed = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!(
                "Remove backend {} and delete its generated directory?",
                backend_name(backend)
            ))
            .default(false)
            .interact()?;
        if !confirmed {
            warn!(shell, "Cancelled backend removal");
            return Ok(());
        }
    }

    header!(shell, "Removing backend: {}", backend_name(backend));

    match backend {
        BackendName::Apple => project.remove_apple_backend().await?,
        BackendName::Android => project.remove_android_backend().await?,
        BackendName::Gtk4 => project.remove_gtk4_backend().await?,
        BackendName::Hydrolysis => project.remove_hydrolysis_backend().await?,
        BackendName::Esp32 => project.remove_esp32_backend().await?,
    }

    success!(shell, "Removed backend {}", backend_name(backend));
    Ok(())
}

fn list_backends(shell: &Shell, project: &Project) {
    header!(shell, "Configured backends");
    let mut configured = 0usize;

    if project.apple_backend().is_some() {
        line!(shell, "  - apple");
        configured += 1;
    }
    if project.android_backend().is_some() {
        line!(shell, "  - android");
        configured += 1;
    }
    if project.gtk4_backend().is_some() {
        line!(shell, "  - gtk4");
        configured += 1;
    }
    if project.hydrolysis_backend().is_some() {
        line!(shell, "  - hydrolysis");
        configured += 1;
    }
    if project.esp32_backend().is_some() {
        line!(shell, "  - esp32");
        configured += 1;
    }

    if configured == 0 {
        line!(shell, "  (none)");
    }
}

const fn is_backend_configured(project: &Project, backend: BackendName) -> bool {
    match backend {
        BackendName::Apple => project.apple_backend().is_some(),
        BackendName::Android => project.android_backend().is_some(),
        BackendName::Gtk4 => project.gtk4_backend().is_some(),
        BackendName::Hydrolysis => project.hydrolysis_backend().is_some(),
        BackendName::Esp32 => project.esp32_backend().is_some(),
    }
}

const fn backend_name(backend: BackendName) -> &'static str {
    match backend {
        BackendName::Apple => "apple",
        BackendName::Android => "android",
        BackendName::Gtk4 => "gtk4",
        BackendName::Hydrolysis => "hydrolysis",
        BackendName::Esp32 => "esp32",
    }
}
