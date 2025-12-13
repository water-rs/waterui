//! `water build` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};

use crate::shell::{self, display_output};
use crate::{error, header, success};
use waterui_cli::{
    android::platform::AndroidPlatform, apple::platform::ApplePlatform, build::BuildOptions,
    platform::Platform as _, project::Project, toolchain::Toolchain,
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

/// Target architecture for building.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TargetArch {
    /// ARM64 / `AArch64` (Apple Silicon, modern Android devices).
    Arm64,
    /// `x86_64` (Intel Macs, Android emulators on Intel/AMD).
    X86_64,
    /// `ARMv7` (older 32-bit Android devices).
    Armv7,
    /// x86 (older 32-bit Android emulators).
    X86,
}

/// Arguments for the build command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Target platform to build for.
    #[arg(short, long, value_enum)]
    platform: TargetPlatform,

    /// Target architecture. Defaults to arm64 for iOS/Android, native for macOS/iOS Simulator.
    #[arg(short, long, value_enum)]
    arch: Option<TargetArch>,

    /// Build in release mode (optimized).
    #[arg(long)]
    release: bool,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,

    /// Output directory to copy the built library to.
    /// The library will be copied as `libwaterui_app.a` (Apple) or `libwaterui_app.so` (Android).
    #[arg(long)]
    output_dir: Option<PathBuf>,
}

/// Run the build command.
pub async fn run(args: Args) -> Result<()> {
    let project_path = args
        .path
        .canonicalize()
        .unwrap_or_else(|_| args.path.clone());
    let project = Project::open(&project_path).await?;

    // Build options with optional output directory
    let build_options = if let Some(ref output_dir) = args.output_dir {
        BuildOptions::new(args.release, false).with_output_dir(output_dir)
    } else {
        BuildOptions::new(args.release, false)
    };
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
    let result = display_output(async {
        match (args.platform, args.arch) {
            // iOS physical device - only arm64 supported
            (TargetPlatform::Ios, None | Some(TargetArch::Arm64)) => {
                ApplePlatform::ios().build(&project, build_options).await
            }
            (TargetPlatform::Ios, Some(arch)) => {
                bail!("iOS physical devices only support arm64, not {:?}", arch)
            }

            // iOS Simulator - arm64 (Apple Silicon) or x86_64 (Intel)
            (TargetPlatform::IosSimulator, None) => {
                // Default to native architecture
                ApplePlatform::ios_simulator()
                    .build(&project, build_options)
                    .await
            }
            (TargetPlatform::IosSimulator, Some(TargetArch::Arm64)) => {
                ApplePlatform::ios_simulator_arm64()
                    .build(&project, build_options)
                    .await
            }
            (TargetPlatform::IosSimulator, Some(TargetArch::X86_64)) => {
                ApplePlatform::ios_simulator_x86_64()
                    .build(&project, build_options)
                    .await
            }
            (TargetPlatform::IosSimulator, Some(arch)) => {
                bail!(
                    "iOS Simulator only supports arm64 or x86_64, not {:?}",
                    arch
                )
            }

            // Android - all architectures supported
            (TargetPlatform::Android, None | Some(TargetArch::Arm64)) => {
                AndroidPlatform::arm64()
                    .build(&project, build_options)
                    .await
            }
            (TargetPlatform::Android, Some(TargetArch::X86_64)) => {
                AndroidPlatform::x86_64()
                    .build(&project, build_options)
                    .await
            }
            (TargetPlatform::Android, Some(TargetArch::Armv7)) => {
                AndroidPlatform::from_abi("armeabi-v7a")
                    .build(&project, build_options)
                    .await
            }
            (TargetPlatform::Android, Some(TargetArch::X86)) => {
                AndroidPlatform::from_abi("x86")
                    .build(&project, build_options)
                    .await
            }

            // macOS - arm64 (Apple Silicon) or x86_64 (Intel)
            (TargetPlatform::Macos, None) => {
                // Default to native architecture
                ApplePlatform::macos().build(&project, build_options).await
            }
            (TargetPlatform::Macos, Some(TargetArch::Arm64)) => {
                ApplePlatform::macos_arm64()
                    .build(&project, build_options)
                    .await
            }
            (TargetPlatform::Macos, Some(TargetArch::X86_64)) => {
                ApplePlatform::macos_x86_64()
                    .build(&project, build_options)
                    .await
            }
            (TargetPlatform::Macos, Some(arch)) => {
                bail!("macOS only supports arm64 or x86_64, not {:?}", arch)
            }
        }
    })
    .await;

    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    match result {
        Ok(lib_dir) => {
            success!("Built library at {}", lib_dir.display());
            if let Some(output_dir) = args.output_dir {
                success!("Copied library to {}", output_dir.display());
            }
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
