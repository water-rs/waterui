//! `water clean` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::Result;

use crate::shell;
use crate::{header, success};
use waterui_cli::{
    android::platform::clean_android, apple::platform::clean_apple, gtk4::platform::clean_gtk4,
    project::Project,
};

/// Target backend for cleaning.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TargetBackend {
    /// Apple backend (iOS/macOS).
    Apple,
    /// Android backend.
    Android,
    /// GTK4 backend (Linux/macOS/Windows).
    Gtk4,
    /// All backends.
    All,
}

/// Arguments for the clean command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Target backend to clean (defaults to all).
    #[arg(short, long, value_enum, default_value = "all")]
    backend: TargetBackend,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

/// Run the clean command.
pub async fn run(args: Args) -> Result<()> {
    let project_path = args
        .path
        .canonicalize()
        .unwrap_or_else(|_| args.path.clone());
    let project = Project::open(&project_path).await?;

    header!("Cleaning build artifacts...");

    match args.backend {
        TargetBackend::All => {
            let spinner = shell::spinner("Cleaning all build artifacts...");
            project.clean_all().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Cleaned all build artifacts");
        }
        TargetBackend::Apple => {
            let spinner = shell::spinner("Cleaning Apple build artifacts...");
            clean_apple(&project).await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Cleaned Apple build artifacts");
        }
        TargetBackend::Android => {
            let spinner = shell::spinner("Cleaning Android build artifacts...");
            clean_android(&project).await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Cleaned Android build artifacts");
        }
        TargetBackend::Gtk4 => {
            let spinner = shell::spinner("Cleaning GTK4 build artifacts...");
            clean_gtk4(&project).await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Cleaned GTK4 build artifacts");
        }
    }

    Ok(())
}
