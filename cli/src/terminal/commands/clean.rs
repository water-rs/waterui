//! `water clean` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::Result;

use crate::shell;
use crate::{header, success};
use waterui_cli::{
    android::platform::AndroidPlatform, apple::platform::ApplePlatform, project::Project,
};

/// Target platform for cleaning.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TargetPlatform {
    /// iOS/macOS (Apple platforms).
    Apple,
    /// Android.
    Android,
    /// All platforms.
    All,
}

/// Arguments for the clean command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Target platform to clean (defaults to all).
    #[arg(short, long, value_enum, default_value = "all")]
    platform: TargetPlatform,

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

    match args.platform {
        TargetPlatform::All => {
            let spinner = shell::spinner("Cleaning all build artifacts...");
            project.clean_all().await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Cleaned all build artifacts");
        }
        TargetPlatform::Apple => {
            let spinner = shell::spinner("Cleaning Apple build artifacts...");
            let platform = ApplePlatform::macos();
            project.clean(platform).await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Cleaned Apple build artifacts");
        }
        TargetPlatform::Android => {
            let spinner = shell::spinner("Cleaning Android build artifacts...");
            let platform = AndroidPlatform::arm64();
            project.clean(platform).await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Cleaned Android build artifacts");
        }
    }

    Ok(())
}
