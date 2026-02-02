//! `water build` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};

use crate::shell::{self, display_output};
use crate::toolchain_checks;
use crate::{error, header, success};
use waterui_cli::{
    android::platform::{AndroidAbi, AndroidPlatform},
    apple::platform::build_rust_lib,
    apple::toolchain::AppleSdk,
    build::BuildOptions,
    platform::TargetPlatform as LibTargetPlatform,
    project::Project,
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

#[allow(clippy::too_many_lines)]
/// Run the build command.
pub async fn run(args: Args) -> Result<()> {
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let project = Project::open(&project_path).await?;

    // Build options with optional output directory
    let build_options = if let Some(ref output_dir) = args.output_dir {
        BuildOptions::new(args.release).with_output_dir(output_dir)
    } else {
        BuildOptions::new(args.release)
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
    match args.platform {
        TargetPlatform::Ios => toolchain_checks::check_apple(AppleSdk::Ios).await?,
        TargetPlatform::IosSimulator => {
            toolchain_checks::check_apple(AppleSdk::IosSimulator).await?
        }
        TargetPlatform::Macos => toolchain_checks::check_apple(AppleSdk::Macos).await?,
        TargetPlatform::Android => toolchain_checks::check_android().await?,
    }
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
                build_rust_lib(&project, LibTargetPlatform::IOS, build_options).await
            }
            (TargetPlatform::Ios, Some(arch)) => {
                bail!("iOS physical devices only support arm64, not {:?}", arch)
            }

            // iOS Simulator - uses host architecture by default
            (
                TargetPlatform::IosSimulator,
                None | Some(TargetArch::Arm64) | Some(TargetArch::X86_64),
            ) => build_rust_lib(&project, LibTargetPlatform::IOSSimulator, build_options).await,
            (TargetPlatform::IosSimulator, Some(arch)) => {
                bail!(
                    "iOS Simulator only supports arm64 or x86_64, not {:?}",
                    arch
                )
            }

            // Android - currently only arm64 supported via TargetPlatform::Android
            (TargetPlatform::Android, arch) => {
                let abi = android_abi(arch.unwrap_or(TargetArch::Arm64));
                AndroidPlatform::new(abi)
                    .build(&project, build_options)
                    .await
            }

            // macOS - uses host architecture
            (TargetPlatform::Macos, None | Some(TargetArch::Arm64) | Some(TargetArch::X86_64)) => {
                build_rust_lib(&project, LibTargetPlatform::MacOS, build_options).await
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

const fn android_abi(arch: TargetArch) -> AndroidAbi {
    match arch {
        TargetArch::Arm64 => AndroidAbi::Arm64V8a,
        TargetArch::X86_64 => AndroidAbi::X86_64,
        TargetArch::Armv7 => AndroidAbi::ArmeabiV7a,
        TargetArch::X86 => AndroidAbi::X86,
    }
}

const fn platform_name(platform: TargetPlatform) -> &'static str {
    match platform {
        TargetPlatform::Ios => "iOS",
        TargetPlatform::IosSimulator => "iOS Simulator",
        TargetPlatform::Android => "Android",
        TargetPlatform::Macos => "macOS",
    }
}
