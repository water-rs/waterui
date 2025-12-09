//! `water build` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{bail, Result};

use crate::shell;
use crate::{error, header, success};
use waterui_cli::{
    android::platform::AndroidPlatform,
    apple::platform::ApplePlatform,
    build::BuildOptions,
    project::Project,
    toolchain::Toolchain,
};

/// Target platform for building.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TargetPlatform {
    /// iOS (physical device).
    Ios,
    /// iOS Simulator.
    IosSimulator,
    /// Android.
    Android,
    /// macOS.
    Macos,
}

/// Arguments for the build command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Target platform to build for.
    #[arg(short, long, value_enum)]
    platform: TargetPlatform,

    /// Build in release mode (optimized).
    #[arg(long)]
    release: bool,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

/// Run the build command.
pub async fn run(args: Args) -> Result<()> {
    let project_path = args
        .path
        .canonicalize()
        .unwrap_or_else(|_| args.path.clone());
    let project = Project::open(&project_path).await?;

    let build_options = BuildOptions::new(args.release);
    let mode = if args.release { "release" } else { "debug" };

    header!(
        "Building {} for {} ({})",
        project.crate_name(),
        platform_name(args.platform),
        mode
    );

    // Step 1: Check toolchain
    let spinner = shell::spinner("Checking toolchain...");
    check_toolchain(args.platform).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Toolchain ready");

    // Step 2: Build
    let spinner = shell::spinner("Compiling Rust library...");
    let result = match args.platform {
        TargetPlatform::Ios => {
            let platform = ApplePlatform::ios();
            project.build(platform, build_options).await
        }
        TargetPlatform::IosSimulator => {
            let platform = ApplePlatform::ios_simulator();
            project.build(platform, build_options).await
        }
        TargetPlatform::Android => {
            let platform = AndroidPlatform::arm64();
            project.build(platform, build_options).await
        }
        TargetPlatform::Macos => {
            let platform = ApplePlatform::macos();
            project.build(platform, build_options).await
        }
    };

    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    match result {
        Ok(lib_path) => {
            success!("Built library at {}", lib_path.display());
            Ok(())
        }
        Err(e) => {
            error!("Build failed: {e}");
            Err(e)
        }
    }
}

async fn check_toolchain(platform: TargetPlatform) -> Result<()> {
    use waterui_cli::platform::Platform;

    match platform {
        TargetPlatform::Ios | TargetPlatform::IosSimulator | TargetPlatform::Macos => {
            let platform = ApplePlatform::ios_simulator();
            let toolchain = platform.toolchain();
            if let Err(e) = toolchain.check().await {
                bail!("Toolchain check failed: {e}");
            }
        }
        TargetPlatform::Android => {
            let platform = AndroidPlatform::arm64();
            let toolchain = platform.toolchain();
            if let Err(e) = toolchain.check().await {
                bail!("Toolchain check failed: {e}");
            }
        }
    }
    Ok(())
}

const fn platform_name(platform: TargetPlatform) -> &'static str {
    match platform {
        TargetPlatform::Ios => "iOS",
        TargetPlatform::IosSimulator => "iOS Simulator",
        TargetPlatform::Android => "Android",
        TargetPlatform::Macos => "macOS",
    }
}
