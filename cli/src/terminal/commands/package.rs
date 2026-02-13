//! `water package` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};

use crate::shell::{self, display_output};
use crate::toolchain_checks;
use crate::{header, success};
use waterui_cli::{
    android::platform::{AndroidAbi, AndroidPlatform},
    apple::platform::{build_rust_lib, package_apple},
    apple::toolchain::AppleSdk,
    build::BuildOptions,
    platform::{PackageOptions, TargetPlatform as LibTargetPlatform},
    project::Project,
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
    const fn to_abi(self) -> AndroidAbi {
        match self {
            Self::Arm64 => AndroidAbi::Arm64V8a,
            Self::X86_64 => AndroidAbi::X86_64,
            Self::Armv7 => AndroidAbi::ArmeabiV7a,
            Self::X86 => AndroidAbi::X86,
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
    validate_arch_args(args.platform, &args.arch)?;

    let project_path = crate::project_path::canonicalize(&args.path)?;
    let project = Project::open(&project_path).await?;

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

    // Step 2: Build (package requires a built library)
    let build_options = BuildOptions::new(args.release);

    if args.platform == TargetPlatform::Android {
        // Clean stale jniLibs before building
        AndroidPlatform::clean_jni_libs(&project).await?;

        // Build for each specified architecture using the AndroidPlatform helper
        for arch in &args.arch {
            let abi = arch.to_abi();
            let spinner = shell::spinner(format!("Building Rust library ({})...", abi.as_str()));
            let platform = AndroidPlatform::new(abi);
            display_output(platform.build(&project, build_options.clone())).await?;
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!("Built for {}", abi.as_str());
        }
    } else {
        let spinner = shell::spinner("Building Rust library...");
        let platform = NonAndroidTargetPlatform::try_from(args.platform)?;
        display_output(build_for_platform(&project, platform, build_options)).await?;
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
            let abis: Vec<AndroidAbi> = args.arch.iter().map(|a| a.to_abi()).collect();
            display_output(AndroidPlatform::package_with_abis(
                &project,
                package_options,
                &abis,
            ))
            .await?
        }
        _ => {
            let platform = NonAndroidTargetPlatform::try_from(args.platform)?;
            display_output(package_for_platform(&project, platform, package_options)).await?
        }
    };

    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Packaged at {}", artifact.path().display());

    Ok(())
}

fn validate_arch_args(platform: TargetPlatform, arch: &[AndroidArch]) -> Result<()> {
    if platform == TargetPlatform::Android && arch.is_empty() {
        bail!(
            "Android platform requires --arch flag.\n\
             Examples:\n  \
             water package --platform android --arch arm64\n  \
             water package --platform android --arch arm64,x86_64"
        );
    }

    if platform != TargetPlatform::Android && !arch.is_empty() {
        bail!("--arch is only valid when --platform android");
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NonAndroidTargetPlatform {
    Ios,
    IosSimulator,
    Macos,
}

impl TryFrom<TargetPlatform> for NonAndroidTargetPlatform {
    type Error = color_eyre::Report;

    fn try_from(platform: TargetPlatform) -> Result<Self> {
        match platform {
            TargetPlatform::Ios => Ok(Self::Ios),
            TargetPlatform::IosSimulator => Ok(Self::IosSimulator),
            TargetPlatform::Macos => Ok(Self::Macos),
            TargetPlatform::Android => Err(color_eyre::eyre::eyre!(
                "Internal error: Android platform passed to non-Android packaging path"
            )),
        }
    }
}

async fn build_for_platform(
    project: &Project,
    platform: NonAndroidTargetPlatform,
    options: BuildOptions,
) -> Result<PathBuf> {
    match platform {
        NonAndroidTargetPlatform::Ios => {
            build_rust_lib(project, LibTargetPlatform::IOS, options).await
        }
        NonAndroidTargetPlatform::IosSimulator => {
            build_rust_lib(project, LibTargetPlatform::IOSSimulator, options).await
        }
        NonAndroidTargetPlatform::Macos => {
            build_rust_lib(project, LibTargetPlatform::MacOS, options).await
        }
    }
}

async fn package_for_platform(
    project: &Project,
    platform: NonAndroidTargetPlatform,
    options: PackageOptions,
) -> Result<waterui_cli::device::Artifact> {
    match platform {
        NonAndroidTargetPlatform::Ios => {
            package_apple(project, LibTargetPlatform::IOS, options).await
        }
        NonAndroidTargetPlatform::IosSimulator => {
            package_apple(project, LibTargetPlatform::IOSSimulator, options).await
        }
        NonAndroidTargetPlatform::Macos => {
            package_apple(project, LibTargetPlatform::MacOS, options).await
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

#[cfg(test)]
mod tests {
    use super::{AndroidArch, TargetPlatform, validate_arch_args};

    #[test]
    fn rejects_empty_arch_for_android() {
        assert!(validate_arch_args(TargetPlatform::Android, &[]).is_err());
    }

    #[test]
    fn rejects_arch_for_non_android() {
        let err = validate_arch_args(TargetPlatform::Ios, &[AndroidArch::Arm64])
            .expect_err("non-android --arch should fail");
        assert!(err.to_string().contains("--arch is only valid"));
    }

    #[test]
    fn accepts_android_arch_values() {
        assert!(validate_arch_args(TargetPlatform::Android, &[AndroidArch::Arm64]).is_ok());
    }
}
