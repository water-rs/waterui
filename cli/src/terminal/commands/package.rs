//! `water package` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};

use crate::shell::{self, display_output};
use crate::{header, success};
use waterui_cli::{
    android::platform::AndroidPlatform, apple::platform::ApplePlatform, build::BuildOptions,
    platform::PackageOptions, project::Project, toolchain::Toolchain,
};

/// Target platform for packaging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
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

/// Target architecture for Android builds.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AndroidArch {
    /// ARM64 (arm64-v8a) - modern Android devices
    Arm64,
    /// x86_64 - emulators on Intel/AMD
    X86_64,
    /// ARMv7 (armeabi-v7a) - older 32-bit devices
    Armv7,
    /// x86 - older 32-bit emulators
    X86,
}

impl AndroidArch {
    /// Convert to Android ABI string.
    const fn to_abi(self) -> &'static str {
        match self {
            Self::Arm64 => "arm64-v8a",
            Self::X86_64 => "x86_64",
            Self::Armv7 => "armeabi-v7a",
            Self::X86 => "x86",
        }
    }
}

/// Arguments for the package command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Target platform to package for.
    #[arg(short, long, value_enum)]
    platform: TargetPlatform,

    /// Build in release mode (optimized).
    #[arg(long)]
    release: bool,

    /// Package for store distribution (App Store, Play Store).
    #[arg(long)]
    distribution: bool,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,

    /// Target architectures for Android (comma-separated).
    /// Examples: --arch arm64, --arch arm64,x86_64
    /// Required for Android platform.
    #[arg(long, value_enum, value_delimiter = ',')]
    arch: Vec<AndroidArch>,
}

/// Run the package command.
pub async fn run(args: Args) -> Result<()> {
    let project_path = args
        .path
        .canonicalize()
        .unwrap_or_else(|_| args.path.clone());
    let project = Project::open(&project_path).await?;

    // Validate --arch flag for Android
    if args.platform == TargetPlatform::Android && args.arch.is_empty() {
        bail!(
            "Android platform requires --arch flag.\n\
             Examples:\n  \
             water package --platform android --arch arm64\n  \
             water package --platform android --arch arm64,x86_64"
        );
    }

    let mode = if args.release { "release" } else { "debug" };
    let dist = if args.distribution {
        " (distribution)"
    } else {
        ""
    };

    header!(
        "Packaging {} for {} ({}){}",
        project.crate_name(),
        platform_name(args.platform),
        mode,
        dist
    );

    // Step 1: Check toolchain
    let spinner = shell::spinner("Checking toolchain...");
    check_toolchain(args.platform).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Toolchain ready");

    // Step 2: Build (package requires a built library)
    let build_options = BuildOptions::new(args.release);

    match args.platform {
        TargetPlatform::Android => {
            // Clean stale jniLibs before building
            AndroidPlatform::clean_jni_libs(&project).await?;

            // Build for each specified architecture
            for arch in &args.arch {
                let spinner =
                    shell::spinner(&format!("Building Rust library ({})...", arch.to_abi()));
                let platform = AndroidPlatform::from_abi(arch.to_abi());
                display_output(project.build(platform, build_options.clone())).await?;
                if let Some(pb) = spinner {
                    pb.finish_and_clear();
                }
                success!("Built for {}", arch.to_abi());
            }
        }
        _ => {
            let spinner = shell::spinner("Building Rust library...");
            display_output(build_for_platform(&project, args.platform, build_options)).await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Built Rust library");
        }
    }

    // Step 3: Package
    let spinner = shell::spinner("Packaging application...");
    let package_options = PackageOptions::new(args.distribution, !args.release);
    let artifact = display_output(package_for_platform(
        &project,
        args.platform,
        package_options,
    ))
    .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Packaged at {}", artifact.path().display());

    Ok(())
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

async fn build_for_platform(
    project: &Project,
    platform: TargetPlatform,
    options: BuildOptions,
) -> Result<PathBuf> {
    match platform {
        TargetPlatform::Ios => {
            let p = ApplePlatform::ios();
            Ok(project.build(p, options).await?)
        }
        TargetPlatform::IosSimulator => {
            let p = ApplePlatform::ios_simulator();
            Ok(project.build(p, options).await?)
        }
        TargetPlatform::Android => {
            let p = AndroidPlatform::arm64();
            Ok(project.build(p, options).await?)
        }
        TargetPlatform::Macos => {
            let p = ApplePlatform::macos();
            Ok(project.build(p, options).await?)
        }
    }
}

async fn package_for_platform(
    project: &Project,
    platform: TargetPlatform,
    options: PackageOptions,
) -> Result<waterui_cli::device::Artifact> {
    match platform {
        TargetPlatform::Ios => {
            let p = ApplePlatform::ios();
            Ok(project.package(p, options).await?)
        }
        TargetPlatform::IosSimulator => {
            let p = ApplePlatform::ios_simulator();
            Ok(project.package(p, options).await?)
        }
        TargetPlatform::Android => {
            let p = AndroidPlatform::arm64();
            Ok(project.package(p, options).await?)
        }
        TargetPlatform::Macos => {
            let p = ApplePlatform::macos();
            Ok(project.package(p, options).await?)
        }
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
