//! `water package` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};

use crate::shell::{self, display_output};
use crate::{header, success};
use waterui_cli::{
    android::platform::{AndroidPlatform, build_android, package_android},
    android::toolchain::{AndroidNdk, AndroidSdk},
    apple::platform::{build_rust_lib, package_apple},
    apple::toolchain::{AppleSdk, Xcode},
    build::BuildOptions,
    platform::{PackageOptions, TargetPlatform as LibTargetPlatform},
    project::Project,
    toolchain::{Toolchain, cmake::Cmake},
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
    /// `x86_64` - emulators on Intel/AMD
    X86_64,
    /// `ARMv7` (armeabi-v7a) - older 32-bit devices
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
    /// Examples: --arch arm64, --arch `arm64,x86_64`
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
    let build_options = BuildOptions::new(args.release, false);

    if args.platform == TargetPlatform::Android {
        // Clean stale jniLibs before building
        AndroidPlatform::clean_jni_libs(&project).await?;

        // Build for each specified architecture using the AndroidPlatform helper
        for arch in &args.arch {
            let spinner = shell::spinner(format!("Building Rust library ({})...", arch.to_abi()));
            let platform = AndroidPlatform::from_abi(arch.to_abi());
            display_output(platform.build(&project, build_options.clone())).await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Built for {}", arch.to_abi());
        }
    } else {
        let spinner = shell::spinner("Building Rust library...");
        display_output(build_for_platform(&project, args.platform, build_options)).await?;
        if let Some(pb) = spinner {
            pb.finish_and_clear();
        }
        success!("Built Rust library");
    }

    // Step 3: Package
    let spinner = shell::spinner("Packaging application...");
    let package_options = PackageOptions::new(args.distribution, !args.release);

    let artifact = match args.platform {
        TargetPlatform::Android => {
            // Use the specialized method that passes all target ABIs to Gradle
            let abis: Vec<&str> = args.arch.iter().map(|a| a.to_abi()).collect();
            display_output(AndroidPlatform::package_with_abis(
                &project,
                package_options,
                &abis,
            ))
            .await?
        }
        _ => {
            display_output(package_for_platform(
                &project,
                args.platform,
                package_options,
            ))
            .await?
        }
    };

    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Packaged at {}", artifact.path().display());

    Ok(())
}

async fn check_toolchain(platform: TargetPlatform) -> Result<()> {
    match platform {
        TargetPlatform::Ios | TargetPlatform::IosSimulator | TargetPlatform::Macos => {
            let xcode = Xcode;
            if let Err(e) = xcode.check().await {
                bail!("Xcode toolchain check failed: {e}");
            }
            let sdk = match platform {
                TargetPlatform::Ios => AppleSdk::Ios,
                TargetPlatform::IosSimulator => AppleSdk::IosSimulator,
                TargetPlatform::Macos => AppleSdk::Macos,
                TargetPlatform::Android => unreachable!(),
            };
            if let Err(e) = sdk.check().await {
                bail!("{sdk} toolchain check failed: {e}");
            }
        }
        TargetPlatform::Android => {
            let sdk = AndroidSdk;
            if let Err(e) = sdk.check().await {
                bail!("Android SDK toolchain check failed: {e}");
            }
            let ndk = AndroidNdk;
            if let Err(e) = ndk.check().await {
                bail!("Android NDK toolchain check failed: {e}");
            }
            let cmake = Cmake {};
            if let Err(e) = cmake.check().await {
                bail!("CMake toolchain check failed: {e}");
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
        TargetPlatform::Ios => build_rust_lib(project, LibTargetPlatform::IOS, options).await,
        TargetPlatform::IosSimulator => {
            build_rust_lib(project, LibTargetPlatform::IOSSimulator, options).await
        }
        TargetPlatform::Android => {
            build_android(project, LibTargetPlatform::Android, options).await
        }
        TargetPlatform::Macos => build_rust_lib(project, LibTargetPlatform::MacOS, options).await,
    }
}

async fn package_for_platform(
    project: &Project,
    platform: TargetPlatform,
    options: PackageOptions,
) -> Result<waterui_cli::device::Artifact> {
    match platform {
        TargetPlatform::Ios => package_apple(project, LibTargetPlatform::IOS, options).await,
        TargetPlatform::IosSimulator => {
            package_apple(project, LibTargetPlatform::IOSSimulator, options).await
        }
        TargetPlatform::Android => {
            package_android(project, LibTargetPlatform::Android, options).await
        }
        TargetPlatform::Macos => package_apple(project, LibTargetPlatform::MacOS, options).await,
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
