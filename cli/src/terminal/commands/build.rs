//! `water build` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};
use target_lexicon::{
    Aarch64Architecture, Architecture, BinaryFormat, Environment, OperatingSystem, Triple, Vendor,
};

use crate::shell::{self, display_output};
use crate::toolchain_checks;
use crate::{error, header, success};
use waterui_cli::{
    android::platform::{AndroidAbi, AndroidPlatform},
    apple::platform::build_rust_lib,
    apple::toolchain::AppleSdk,
    backend::reinit_backend,
    build::BuildOptions,
    gtk4::{backend::Gtk4Backend, platform::build_gtk4},
    hydrolysis::{backend::HydrolysisBackend, platform::build_hydrolysis},
    platform::TargetPlatform as LibTargetPlatform,
    project::{PackageType, Project},
};

/// Target platform for building.
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
    /// Linux.
    Linux,
    /// Windows.
    Windows,
}

/// Target backend for building.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TargetBackend {
    /// Apple backend (UIKit/AppKit).
    Apple,
    /// Android backend.
    Android,
    /// GTK4 backend.
    Gtk4,
    /// Hydrolysis backend.
    Hydrolysis,
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

    /// Backend to use (overrides default for platform).
    #[arg(short, long, value_enum)]
    backend: Option<TargetBackend>,

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
    /// Only valid for Apple/Android backends.
    #[arg(long)]
    output_dir: Option<PathBuf>,
}

/// Run the build command.
pub async fn run(args: Args) -> Result<()> {
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let mut project = Project::open(&project_path).await?;

    if project.package_type() != PackageType::App {
        bail!(
            "`water build` is only supported for app mode projects.\n\
             Playground projects are managed by `water run` and `water package`."
        );
    }

    let backend = resolve_backend(args.platform, args.backend)?;
    validate_desktop_backend_platform_on_host(args.platform, backend)?;
    validate_arch_args(backend, args.arch)?;
    validate_output_dir_args(backend, args.output_dir.as_ref())?;

    // Validate backend is configured in app mode.
    match backend {
        TargetBackend::Apple if project.apple_backend().is_none() => {
            bail!("Apple backend is not configured. Run `water backend add apple`.")
        }
        TargetBackend::Android if project.android_backend().is_none() => {
            bail!("Android backend is not configured. Run `water backend add android`.")
        }
        TargetBackend::Gtk4 if project.gtk4_backend().is_none() => {
            bail!("GTK4 backend is not configured. Run `water backend add gtk4`.")
        }
        TargetBackend::Hydrolysis if project.hydrolysis_backend().is_none() => {
            bail!("Hydrolysis backend is not configured. Run `water backend add hydrolysis`.")
        }
        _ => {}
    }

    // Safety net for generated Rust backends if backend config exists but directory missing.
    match backend {
        TargetBackend::Gtk4 => {
            let cargo_toml = project.backend_path::<Gtk4Backend>().join("Cargo.toml");
            if !cargo_toml.exists() {
                let spinner = shell::spinner("Re-initializing GTK4 backend...");
                reinit_backend::<Gtk4Backend>(&project).await?;
                project = Project::open(&project_path).await?;
                if let Some(pb) = spinner {
                    pb.finish_and_clear();
                }
                success!("GTK4 backend re-initialized");
            }
        }
        TargetBackend::Hydrolysis => {
            let cargo_toml = project
                .backend_path::<HydrolysisBackend>()
                .join("Cargo.toml");
            if !cargo_toml.exists() {
                let spinner = shell::spinner("Re-initializing hydrolysis backend...");
                reinit_backend::<HydrolysisBackend>(&project).await?;
                project = Project::open(&project_path).await?;
                if let Some(pb) = spinner {
                    pb.finish_and_clear();
                }
                success!("Hydrolysis backend re-initialized");
            }
        }
        _ => {}
    }

    let mut build_options = if let Some(ref output_dir) = args.output_dir {
        BuildOptions::new(args.release).with_output_dir(output_dir)
    } else {
        BuildOptions::new(args.release)
    };

    if backend == TargetBackend::Apple
        && let Some(triple) = apple_target_triple_override(args.platform, args.arch)
    {
        build_options = build_options.with_target_triple(triple);
    }

    let mode = if args.release { "release" } else { "debug" };
    header!(
        "Building {} for {} via {} ({})",
        project.crate_name(),
        platform_name(args.platform),
        backend_name(backend),
        mode
    );

    // Step 1: Check toolchain
    let spinner = shell::spinner("Checking toolchain...");
    check_toolchain_for_backend(args.platform, backend).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Toolchain ready");

    // Step 2: Build
    let spinner = shell::spinner("Compiling...");
    let result = display_output(async {
        match backend {
            TargetBackend::Apple => {
                build_for_apple(&project, args.platform, args.arch, build_options).await
            }
            TargetBackend::Android => build_for_android(&project, args.arch, build_options).await,
            TargetBackend::Gtk4 => build_gtk4(&project, build_options).await,
            TargetBackend::Hydrolysis => {
                build_hydrolysis(&project, lib_platform(args.platform), build_options).await
            }
        }
    })
    .await;

    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    match result {
        Ok(output_path) => {
            success!("Build output at {}", output_path.display());
            if let Some(output_dir) = args.output_dir {
                success!("Copied library to {}", output_dir.display());
            }
            Ok(())
        }
        Err(err) => {
            error!("Build failed: {err}");
            Err(err)
        }
    }
}

fn resolve_backend(
    platform: TargetPlatform,
    backend_override: Option<TargetBackend>,
) -> Result<TargetBackend> {
    let default_backend = match platform {
        TargetPlatform::Ios | TargetPlatform::IosSimulator => TargetBackend::Apple,
        TargetPlatform::Android => TargetBackend::Android,
        TargetPlatform::Macos => TargetBackend::Apple,
        TargetPlatform::Linux => TargetBackend::Gtk4,
        TargetPlatform::Windows => TargetBackend::Hydrolysis,
    };
    let backend = backend_override.unwrap_or(default_backend);

    let supported = match (platform, backend) {
        (TargetPlatform::Ios | TargetPlatform::IosSimulator, TargetBackend::Apple) => true,
        (TargetPlatform::Android, TargetBackend::Android) => true,
        (TargetPlatform::Macos, TargetBackend::Apple) => true,
        (TargetPlatform::Macos, TargetBackend::Hydrolysis) => true,
        (TargetPlatform::Linux, TargetBackend::Gtk4 | TargetBackend::Hydrolysis) => true,
        (TargetPlatform::Windows, TargetBackend::Hydrolysis) => true,
        _ => false,
    };
    if !supported {
        bail!(
            "Backend {:?} does not support platform {:?}.\n\
             Valid combinations:\n  \
             - iOS/iOS Simulator: apple\n  \
             - Android: android\n  \
             - macOS: apple, hydrolysis\n  \
             - Linux: gtk4, hydrolysis\n  \
             - Windows: hydrolysis",
            backend,
            platform
        );
    }
    Ok(backend)
}

fn validate_arch_args(backend: TargetBackend, arch: Option<TargetArch>) -> Result<()> {
    if matches!(backend, TargetBackend::Gtk4 | TargetBackend::Hydrolysis) && arch.is_some() {
        bail!("--arch is not supported for gtk4/hydrolysis backends");
    }
    Ok(())
}

fn validate_output_dir_args(backend: TargetBackend, output_dir: Option<&PathBuf>) -> Result<()> {
    if output_dir.is_some() && matches!(backend, TargetBackend::Gtk4 | TargetBackend::Hydrolysis) {
        bail!("--output-dir is only supported for Apple/Android backends");
    }
    Ok(())
}

async fn check_toolchain_for_backend(
    platform: TargetPlatform,
    backend: TargetBackend,
) -> Result<()> {
    match backend {
        TargetBackend::Apple => {
            let sdk = match platform {
                TargetPlatform::Ios => AppleSdk::Ios,
                TargetPlatform::IosSimulator => AppleSdk::IosSimulator,
                TargetPlatform::Macos => AppleSdk::Macos,
                TargetPlatform::Android | TargetPlatform::Linux | TargetPlatform::Windows => {
                    bail!("Internal error: Apple backend is not supported on {platform:?}")
                }
            };
            toolchain_checks::check_apple(sdk).await?;
        }
        TargetBackend::Android => {
            if platform != TargetPlatform::Android {
                bail!("Internal error: Android backend is not supported on {platform:?}");
            }
            toolchain_checks::check_android_build_or_package().await?;
        }
        TargetBackend::Gtk4 => {
            if platform != TargetPlatform::Linux {
                bail!("Internal error: GTK4 backend is not supported on {platform:?}");
            }
            toolchain_checks::check_gtk4().await?;
        }
        TargetBackend::Hydrolysis => {
            if platform != TargetPlatform::Macos
                && platform != TargetPlatform::Linux
                && platform != TargetPlatform::Windows
            {
                bail!("Internal error: hydrolysis backend is not supported on {platform:?}");
            }
        }
    }
    Ok(())
}

async fn build_for_apple(
    project: &Project,
    platform: TargetPlatform,
    arch: Option<TargetArch>,
    options: BuildOptions,
) -> Result<PathBuf> {
    match (platform, arch) {
        (TargetPlatform::Ios, None | Some(TargetArch::Arm64)) => {
            build_rust_lib(project, LibTargetPlatform::IOS, options).await
        }
        (TargetPlatform::Ios, Some(target_arch)) => {
            bail!(
                "iOS physical devices only support arm64, not {:?}",
                target_arch
            )
        }
        (
            TargetPlatform::IosSimulator,
            None | Some(TargetArch::Arm64) | Some(TargetArch::X86_64),
        ) => build_rust_lib(project, LibTargetPlatform::IOSSimulator, options).await,
        (TargetPlatform::IosSimulator, Some(target_arch)) => {
            bail!(
                "iOS Simulator only supports arm64 or x86_64, not {:?}",
                target_arch
            )
        }
        (TargetPlatform::Macos, None | Some(TargetArch::Arm64) | Some(TargetArch::X86_64)) => {
            build_rust_lib(project, LibTargetPlatform::MacOS, options).await
        }
        (TargetPlatform::Macos, Some(target_arch)) => {
            bail!("macOS only supports arm64 or x86_64, not {:?}", target_arch)
        }
        (TargetPlatform::Android | TargetPlatform::Linux | TargetPlatform::Windows, _) => {
            bail!(
                "Internal error: invalid Apple backend platform {:?}",
                platform
            )
        }
    }
}

async fn build_for_android(
    project: &Project,
    arch: Option<TargetArch>,
    options: BuildOptions,
) -> Result<PathBuf> {
    let abi = android_abi(arch.unwrap_or(TargetArch::Arm64));
    AndroidPlatform::new(abi).build(project, options).await
}

fn validate_desktop_backend_platform_on_host(
    platform: TargetPlatform,
    backend: TargetBackend,
) -> Result<()> {
    match backend {
        TargetBackend::Gtk4 => {
            #[cfg(target_os = "linux")]
            {
                if platform != TargetPlatform::Linux {
                    bail!("GTK4 backend on Linux host requires --platform linux");
                }
            }

            #[cfg(not(target_os = "linux"))]
            {
                bail!("GTK4 backend is only supported on Linux hosts");
            }
        }
        TargetBackend::Hydrolysis => {
            #[cfg(target_os = "macos")]
            if platform != TargetPlatform::Macos {
                bail!("Hydrolysis backend on macOS host requires --platform macos");
            }

            #[cfg(target_os = "linux")]
            if platform != TargetPlatform::Linux {
                bail!("Hydrolysis backend on Linux host requires --platform linux");
            }

            #[cfg(target_os = "windows")]
            if platform != TargetPlatform::Windows {
                bail!("Hydrolysis backend on Windows host requires --platform windows");
            }

            #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
            bail!("Hydrolysis backend is only supported on macOS, Linux, or Windows hosts");
        }
        TargetBackend::Apple => {
            #[cfg(not(target_os = "macos"))]
            bail!("Apple backend requires a macOS host");
        }
        TargetBackend::Android => {}
    }

    Ok(())
}

const fn lib_platform(platform: TargetPlatform) -> LibTargetPlatform {
    match platform {
        TargetPlatform::Ios => LibTargetPlatform::IOS,
        TargetPlatform::IosSimulator => LibTargetPlatform::IOSSimulator,
        TargetPlatform::Android => LibTargetPlatform::Android,
        TargetPlatform::Macos => LibTargetPlatform::MacOS,
        TargetPlatform::Linux => LibTargetPlatform::Linux,
        TargetPlatform::Windows => LibTargetPlatform::Windows,
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
        TargetPlatform::Linux => "Linux",
        TargetPlatform::Windows => "Windows",
    }
}

const fn backend_name(backend: TargetBackend) -> &'static str {
    match backend {
        TargetBackend::Apple => "Apple",
        TargetBackend::Android => "Android",
        TargetBackend::Gtk4 => "GTK4",
        TargetBackend::Hydrolysis => "Hydrolysis",
    }
}

fn apple_target_triple_override(
    platform: TargetPlatform,
    arch: Option<TargetArch>,
) -> Option<Triple> {
    match (platform, arch) {
        (TargetPlatform::Macos, Some(TargetArch::Arm64)) => Some(Triple {
            architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
            vendor: Vendor::Apple,
            operating_system: OperatingSystem::Darwin(None),
            environment: Environment::Unknown,
            binary_format: BinaryFormat::Macho,
        }),
        (TargetPlatform::Macos, Some(TargetArch::X86_64)) => Some(Triple {
            architecture: Architecture::X86_64,
            vendor: Vendor::Apple,
            operating_system: OperatingSystem::Darwin(None),
            environment: Environment::Unknown,
            binary_format: BinaryFormat::Macho,
        }),
        (TargetPlatform::IosSimulator, Some(TargetArch::Arm64)) => Some(Triple {
            architecture: Architecture::Aarch64(Aarch64Architecture::Aarch64),
            vendor: Vendor::Apple,
            operating_system: OperatingSystem::IOS(None),
            environment: Environment::Sim,
            binary_format: BinaryFormat::Macho,
        }),
        (TargetPlatform::IosSimulator, Some(TargetArch::X86_64)) => Some(Triple {
            architecture: Architecture::X86_64,
            vendor: Vendor::Apple,
            operating_system: OperatingSystem::IOS(None),
            environment: Environment::Unknown,
            binary_format: BinaryFormat::Macho,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{TargetBackend, TargetPlatform, resolve_backend, validate_output_dir_args};

    #[test]
    fn resolve_backend_defaults_match_platforms() {
        assert_eq!(
            resolve_backend(TargetPlatform::Ios, None).expect("ios backend"),
            TargetBackend::Apple
        );
        assert_eq!(
            resolve_backend(TargetPlatform::Android, None).expect("android backend"),
            TargetBackend::Android
        );
        assert_eq!(
            resolve_backend(TargetPlatform::Linux, None).expect("linux backend"),
            TargetBackend::Gtk4
        );
        assert_eq!(
            resolve_backend(TargetPlatform::Windows, None).expect("windows backend"),
            TargetBackend::Hydrolysis
        );
    }

    #[test]
    fn output_dir_rejected_for_desktop_backends() {
        let output = Some(&std::path::PathBuf::from("/tmp/out"));
        assert!(validate_output_dir_args(TargetBackend::Gtk4, output).is_err());
        assert!(validate_output_dir_args(TargetBackend::Hydrolysis, output).is_err());
        assert!(validate_output_dir_args(TargetBackend::Apple, output).is_ok());
    }
}
