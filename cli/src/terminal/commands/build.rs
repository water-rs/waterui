//! `water build` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};
use target_lexicon::{
    Aarch64Architecture, Architecture, BinaryFormat, Environment, OperatingSystem, Triple, Vendor,
};

use crate::shell::Shell;
use crate::toolchain_checks;
use crate::{error, header, success};
use waterui_cli::{
    android::platform::{AndroidAbi, AndroidPlatform},
    apple::platform::build_rust_lib,
    apple::toolchain::AppleSdk,
    backend::reinit_backend,
    build::BuildOptions,
    esp32::{backend::Esp32Backend, platform::build_esp32},
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
    /// ESP32-S3 (Xtensa firmware).
    Esp32s3,
    /// ESP32-C3 (RISC-V firmware).
    Esp32c3,
}

impl TargetPlatform {
    /// The ESP32 chip a platform selects, if it is an ESP32 platform.
    const fn esp32_chip(self) -> Option<waterui_cli::esp32::chip::Esp32Chip> {
        use waterui_cli::esp32::chip::Esp32Chip;
        match self {
            Self::Esp32s3 => Some(Esp32Chip::Esp32S3),
            Self::Esp32c3 => Some(Esp32Chip::Esp32C3),
            _ => None,
        }
    }
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
    /// Dew backend (ESP32 firmware).
    Dew,
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

struct BuildContext {
    project: Project,
    backend: TargetBackend,
    build_options: BuildOptions,
}

/// Run the build command.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    let context = prepare_build_context(shell, &args).await?;
    print_build_header(
        shell,
        &context.project,
        args.platform,
        context.backend,
        args.release,
    );
    check_build_toolchain(shell, args.platform, context.backend, args.arch).await?;
    let result = execute_build(shell, &args, &context).await;

    handle_build_result(shell, result, args.output_dir)
}

async fn prepare_build_context(shell: &Shell, args: &Args) -> Result<BuildContext> {
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let mut project = Project::open(&project_path).await?;
    ensure_app_project(&project)?;

    let backend = resolve_and_validate_backend(args)?;
    ensure_backend_configured(&project, backend)?;

    // Selecting an ESP32 platform pins the chip so the generated harness and
    // build target follow the platform.
    if let Some(chip) = args.platform.esp32_chip() {
        project.set_esp32_chip(chip).await?;
    }

    let project = ensure_generated_backend_ready(shell, &project_path, project, backend).await?;
    let build_options = build_options(args, backend);

    Ok(BuildContext {
        project,
        backend,
        build_options,
    })
}

fn ensure_app_project(project: &Project) -> Result<()> {
    if project.package_type() != PackageType::App {
        bail!(
            "`water build` is only supported for app mode projects.\n\
             Playground projects are managed by `water run` and `water package`."
        );
    }
    Ok(())
}

fn resolve_and_validate_backend(args: &Args) -> Result<TargetBackend> {
    let backend = resolve_backend(args.platform, args.backend)?;
    validate_desktop_backend_platform_on_host(args.platform, backend)?;
    validate_arch_args(backend, args.arch)?;
    validate_output_dir_args(backend, args.output_dir.as_ref())?;
    Ok(backend)
}

fn ensure_backend_configured(project: &Project, backend: TargetBackend) -> Result<()> {
    match backend {
        TargetBackend::Apple if project.apple_backend().is_none() => {
            bail!("Apple backend is not configured. Run `water backend add apple`.");
        }
        TargetBackend::Android if project.android_backend().is_none() => {
            bail!("Android backend is not configured. Run `water backend add android`.");
        }
        TargetBackend::Gtk4 if project.gtk4_backend().is_none() => {
            bail!("GTK4 backend is not configured. Run `water backend add gtk4`.");
        }
        TargetBackend::Hydrolysis if project.hydrolysis_backend().is_none() => {
            bail!("Hydrolysis backend is not configured. Run `water backend add hydrolysis`.");
        }
        TargetBackend::Dew if project.esp32_backend().is_none() => {
            bail!("ESP32 backend is not configured. Run `water backend add esp32`.");
        }
        _ => Ok(()),
    }
}

async fn ensure_generated_backend_ready(
    shell: &Shell,
    project_path: &PathBuf,
    project: Project,
    backend: TargetBackend,
) -> Result<Project> {
    match backend {
        TargetBackend::Gtk4
            if !project
                .backend_path::<Gtk4Backend>()
                .join("Cargo.toml")
                .exists() =>
        {
            reinitialize_generated_backend::<Gtk4Backend>(
                shell,
                project_path,
                &project,
                "Re-initializing GTK4 backend...",
                "GTK4 backend re-initialized",
            )
            .await
        }
        TargetBackend::Hydrolysis
            if !project
                .backend_path::<HydrolysisBackend>()
                .join("Cargo.toml")
                .exists() =>
        {
            reinitialize_generated_backend::<HydrolysisBackend>(
                shell,
                project_path,
                &project,
                "Re-initializing hydrolysis backend...",
                "Hydrolysis backend re-initialized",
            )
            .await
        }
        TargetBackend::Dew if Esp32Backend::requires_regeneration(&project)? => {
            reinitialize_generated_backend::<Esp32Backend>(
                shell,
                project_path,
                &project,
                "Re-initializing ESP32 backend...",
                "ESP32 backend re-initialized",
            )
            .await
        }
        _ => Ok(project),
    }
}

async fn reinitialize_generated_backend<T>(
    shell: &Shell,
    project_path: &PathBuf,
    project: &Project,
    spinner_message: &str,
    success_message: &str,
) -> Result<Project>
where
    T: waterui_cli::backend::Backend,
{
    let spinner = shell.spinner(spinner_message);
    reinit_backend::<T>(project).await?;
    let project = Project::open(project_path).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "{success_message}");
    Ok(project)
}

fn build_options(args: &Args, backend: TargetBackend) -> BuildOptions {
    let mut build_options = args.output_dir.as_ref().map_or_else(
        || BuildOptions::development(args.release),
        |output_dir| BuildOptions::development(args.release).with_output_dir(output_dir),
    );

    if backend == TargetBackend::Apple
        && let Some(triple) = apple_target_triple_override(args.platform, args.arch)
    {
        build_options = build_options.with_target_triple(triple);
    }

    build_options
}

fn print_build_header(
    shell: &Shell,
    project: &Project,
    platform: TargetPlatform,
    backend: TargetBackend,
    release: bool,
) {
    let mode = if release { "release" } else { "debug" };
    header!(
        shell,
        "Building {} for {} via {} ({})",
        project.crate_name(),
        platform_name(platform),
        backend_name(backend),
        mode
    );
}

async fn check_build_toolchain(
    shell: &Shell,
    platform: TargetPlatform,
    backend: TargetBackend,
    arch: Option<TargetArch>,
) -> Result<()> {
    let spinner = shell.spinner("Checking toolchain...");
    check_toolchain_for_backend(platform, backend, arch).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "Toolchain ready");
    Ok(())
}

async fn execute_build(shell: &Shell, args: &Args, context: &BuildContext) -> Result<PathBuf> {
    let spinner = shell.spinner("Compiling...");
    let result = shell
        .display_output(async {
            match context.backend {
                TargetBackend::Apple => {
                    build_for_apple(
                        &context.project,
                        args.platform,
                        args.arch,
                        context.build_options.clone(),
                    )
                    .await
                }
                TargetBackend::Android => {
                    build_for_android(&context.project, args.arch, context.build_options.clone())
                        .await
                }
                TargetBackend::Gtk4 => {
                    build_gtk4(&context.project, context.build_options.clone()).await
                }
                TargetBackend::Hydrolysis => {
                    build_hydrolysis(
                        &context.project,
                        lib_platform(args.platform),
                        context.build_options.clone(),
                    )
                    .await
                }
                TargetBackend::Dew => {
                    build_esp32(&context.project, context.build_options.clone()).await
                }
            }
        })
        .await;

    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    result
}

fn handle_build_result(
    shell: &Shell,
    result: Result<PathBuf>,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    match result {
        Ok(output_path) => {
            success!(shell, "Build output at {}", output_path.display());
            if let Some(output_dir) = output_dir {
                success!(shell, "Copied library to {}", output_dir.display());
            }
            Ok(())
        }
        Err(err) => {
            error!(shell, "Build failed: {err}");
            Err(err)
        }
    }
}

fn resolve_backend(
    platform: TargetPlatform,
    backend_override: Option<TargetBackend>,
) -> Result<TargetBackend> {
    let default_backend = match platform {
        TargetPlatform::Ios | TargetPlatform::IosSimulator | TargetPlatform::Macos => {
            TargetBackend::Apple
        }
        TargetPlatform::Android => TargetBackend::Android,
        TargetPlatform::Linux => TargetBackend::Gtk4,
        TargetPlatform::Windows => TargetBackend::Hydrolysis,
        TargetPlatform::Esp32s3 | TargetPlatform::Esp32c3 => TargetBackend::Dew,
    };
    let backend = backend_override.unwrap_or(default_backend);

    let supported = matches!(
        (platform, backend),
        (
            TargetPlatform::Ios | TargetPlatform::IosSimulator,
            TargetBackend::Apple
        ) | (
            TargetPlatform::Macos,
            TargetBackend::Apple | TargetBackend::Hydrolysis
        ) | (TargetPlatform::Android, TargetBackend::Android)
            | (
                TargetPlatform::Linux,
                TargetBackend::Gtk4 | TargetBackend::Hydrolysis
            )
            | (TargetPlatform::Windows, TargetBackend::Hydrolysis)
            | (
                TargetPlatform::Esp32s3 | TargetPlatform::Esp32c3,
                TargetBackend::Dew
            )
    );
    if !supported {
        bail!(
            "Backend {:?} does not support platform {:?}.\n\
             Valid combinations:\n  \
             - iOS/iOS Simulator: apple\n  \
             - Android: android\n  \
             - macOS: apple, hydrolysis\n  \
             - Linux: gtk4, hydrolysis\n  \
             - Windows: hydrolysis\n  \
             - ESP32-S3: dew\n  \
             - ESP32-C3: dew",
            backend,
            platform
        );
    }
    Ok(backend)
}

fn validate_arch_args(backend: TargetBackend, arch: Option<TargetArch>) -> Result<()> {
    if matches!(
        backend,
        TargetBackend::Gtk4 | TargetBackend::Hydrolysis | TargetBackend::Dew
    ) && arch.is_some()
    {
        bail!("--arch is not supported for gtk4/hydrolysis/dew backends");
    }
    Ok(())
}

fn validate_output_dir_args(backend: TargetBackend, output_dir: Option<&PathBuf>) -> Result<()> {
    if output_dir.is_some()
        && matches!(
            backend,
            TargetBackend::Gtk4 | TargetBackend::Hydrolysis | TargetBackend::Dew
        )
    {
        bail!("--output-dir is only supported for Apple/Android backends");
    }
    Ok(())
}

async fn check_toolchain_for_backend(
    platform: TargetPlatform,
    backend: TargetBackend,
    arch: Option<TargetArch>,
) -> Result<()> {
    match backend {
        TargetBackend::Apple => {
            let sdk = match platform {
                TargetPlatform::Ios => AppleSdk::Ios,
                TargetPlatform::IosSimulator => AppleSdk::IosSimulator,
                TargetPlatform::Macos => AppleSdk::Macos,
                TargetPlatform::Android
                | TargetPlatform::Linux
                | TargetPlatform::Windows
                | TargetPlatform::Esp32s3
                | TargetPlatform::Esp32c3 => {
                    bail!("Internal error: Apple backend is not supported on {platform:?}");
                }
            };
            toolchain_checks::check_apple(sdk).await?;
        }
        TargetBackend::Android => {
            if platform != TargetPlatform::Android {
                bail!("Internal error: Android backend is not supported on {platform:?}");
            }
            let requested_abi = android_abi(arch.unwrap_or(TargetArch::Arm64));
            toolchain_checks::check_android_build_or_package_for_abis(&[requested_abi]).await?;
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
        TargetBackend::Dew => {
            if platform.esp32_chip().is_none() {
                bail!("Internal error: dew backend is not supported on {platform:?}");
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
            );
        }
        (TargetPlatform::IosSimulator, None | Some(TargetArch::Arm64 | TargetArch::X86_64)) => {
            build_rust_lib(project, LibTargetPlatform::IOSSimulator, options).await
        }
        (TargetPlatform::IosSimulator, Some(target_arch)) => {
            bail!(
                "iOS Simulator only supports arm64 or x86_64, not {:?}",
                target_arch
            );
        }
        (TargetPlatform::Macos, None | Some(TargetArch::Arm64 | TargetArch::X86_64)) => {
            build_rust_lib(project, LibTargetPlatform::MacOS, options).await
        }
        (TargetPlatform::Macos, Some(target_arch)) => {
            bail!("macOS only supports arm64 or x86_64, not {:?}", target_arch);
        }
        (
            TargetPlatform::Android
            | TargetPlatform::Linux
            | TargetPlatform::Windows
            | TargetPlatform::Esp32s3
            | TargetPlatform::Esp32c3,
            _,
        ) => {
            bail!(
                "Internal error: invalid Apple backend platform {:?}",
                platform
            );
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
        // The Dew/ESP32 firmware cross-compiles from any host with espup installed.
        TargetBackend::Android | TargetBackend::Dew => {}
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
        TargetPlatform::Esp32s3 => LibTargetPlatform::Esp32S3,
        TargetPlatform::Esp32c3 => LibTargetPlatform::Esp32C3,
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
        TargetPlatform::Esp32s3 => "ESP32-S3",
        TargetPlatform::Esp32c3 => "ESP32-C3",
    }
}

const fn backend_name(backend: TargetBackend) -> &'static str {
    match backend {
        TargetBackend::Apple => "Apple",
        TargetBackend::Android => "Android",
        TargetBackend::Gtk4 => "GTK4",
        TargetBackend::Hydrolysis => "Hydrolysis",
        TargetBackend::Dew => "Dew",
    }
}

const fn apple_target_triple_override(
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
