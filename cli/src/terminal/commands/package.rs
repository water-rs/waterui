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
    backend::reinit_backend,
    build::BuildOptions,
    device::Artifact,
    gtk4::{
        backend::Gtk4Backend,
        platform::{build_gtk4, package_gtk4},
    },
    hydrolysis::{
        backend::HydrolysisBackend,
        platform::{build_hydrolysis, package_hydrolysis},
    },
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
    /// Linux.
    Linux,
    /// Windows.
    Windows,
    /// Web.
    Web,
}

/// Target backend for packaging.
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

    /// Backend to use (overrides default for platform).
    /// Required: `water package` always needs an explicit backend.
    #[arg(short, long, value_enum)]
    backend: TargetBackend,

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
    /// Required when packaging Android backend.
    #[arg(long, value_enum, value_delimiter = ',')]
    arch: Vec<AndroidArch>,
}

struct PackagingContext {
    project: Project,
    backend: TargetBackend,
    build_options: BuildOptions,
}

/// Run the package command.
pub async fn run(args: Args) -> Result<()> {
    let context = prepare_packaging_context(&args).await?;
    print_packaging_header(
        &context.project,
        args.platform,
        context.backend,
        args.release,
        args.distribution,
    );
    check_packaging_toolchain(args.platform, context.backend, &args.arch).await?;
    build_packaging_artifacts(&args, &context).await?;
    package_artifact(&args, &context).await
}

async fn prepare_packaging_context(args: &Args) -> Result<PackagingContext> {
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let project = Project::open(&project_path).await?;
    let backend = resolve_backend(args.platform, args.backend)?;

    validate_arch_args(backend, &args.arch)?;
    validate_desktop_backend_platform_on_host(args.platform, backend)?;
    ensure_packaging_backend_ready(&project, backend)?;
    let project = ensure_packaging_backend_generated(&project_path, project, backend).await?;

    Ok(PackagingContext {
        project,
        backend,
        build_options: BuildOptions::packaging(args.release),
    })
}

fn ensure_packaging_backend_ready(project: &Project, backend: TargetBackend) -> Result<()> {
    if project.is_playground() {
        return Ok(());
    }

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
        _ => Ok(()),
    }
}

async fn ensure_packaging_backend_generated(
    project_path: &PathBuf,
    project: Project,
    backend: TargetBackend,
) -> Result<Project> {
    match backend {
        TargetBackend::Gtk4 if project.is_playground() => {
            let needs_reinit = project.gtk4_backend().is_none()
                || !project
                    .backend_path::<Gtk4Backend>()
                    .join("Cargo.toml")
                    .exists();
            ensure_packaging_generated_backend::<Gtk4Backend>(
                project_path,
                project,
                needs_reinit,
                "Initializing GTK4 backend...",
                "GTK4 backend initialized",
            )
            .await
        }
        TargetBackend::Hydrolysis if project.is_playground() => {
            let needs_reinit = HydrolysisBackend::requires_regeneration(&project)?;
            ensure_packaging_generated_backend::<HydrolysisBackend>(
                project_path,
                project,
                needs_reinit,
                "Initializing hydrolysis backend...",
                "Hydrolysis backend initialized",
            )
            .await
        }
        _ => Ok(project),
    }
}

async fn ensure_packaging_generated_backend<T>(
    project_path: &PathBuf,
    project: Project,
    needs_reinit: bool,
    spinner_message: &str,
    success_message: &str,
) -> Result<Project>
where
    T: waterui_cli::backend::Backend,
{
    if !needs_reinit {
        return Ok(project);
    }

    let spinner = shell::spinner(spinner_message);
    reinit_backend::<T>(&project).await?;
    let project = Project::open(project_path).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("{success_message}");
    Ok(project)
}

fn print_packaging_header(
    project: &Project,
    platform: TargetPlatform,
    backend: TargetBackend,
    release: bool,
    distribution: bool,
) {
    let mode = if release { "release" } else { "debug" };
    let dist = if distribution { " (distribution)" } else { "" };
    header!(
        "Packaging {} for {} via {} ({}){}",
        project.crate_name(),
        platform_name(platform),
        backend_name(backend),
        mode,
        dist
    );
}

async fn check_packaging_toolchain(
    platform: TargetPlatform,
    backend: TargetBackend,
    arch: &[AndroidArch],
) -> Result<()> {
    let spinner = shell::spinner("Checking toolchain...");
    check_toolchain_for_backend(platform, backend, arch).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Toolchain ready");
    Ok(())
}

async fn build_packaging_artifacts(args: &Args, context: &PackagingContext) -> Result<()> {
    match context.backend {
        TargetBackend::Android => {
            build_android_packaging_artifacts(
                &context.project,
                &args.arch,
                context.build_options.clone(),
            )
            .await
        }
        TargetBackend::Apple => {
            build_apple_packaging_artifacts(
                &context.project,
                args.platform,
                context.build_options.clone(),
            )
            .await
        }
        TargetBackend::Gtk4 => {
            build_gtk4_packaging_artifacts(&context.project, context.build_options.clone()).await
        }
        TargetBackend::Hydrolysis => {
            build_hydrolysis_packaging_artifacts(
                &context.project,
                args.platform,
                context.build_options.clone(),
            )
            .await
        }
    }
}

async fn build_android_packaging_artifacts(
    project: &Project,
    arch: &[AndroidArch],
    build_options: BuildOptions,
) -> Result<()> {
    AndroidPlatform::clean_jni_libs(project).await?;
    for arch in arch {
        let abi = arch.to_abi();
        let spinner = shell::spinner(format!("Building Rust library ({})...", abi.as_str()));
        display_output(AndroidPlatform::new(abi).build(project, build_options.clone())).await?;
        if let Some(pb) = spinner {
            pb.finish_and_clear();
        }
        success!("Built for {}", abi.as_str());
    }
    Ok(())
}

async fn build_apple_packaging_artifacts(
    project: &Project,
    platform: TargetPlatform,
    build_options: BuildOptions,
) -> Result<()> {
    let spinner = shell::spinner("Building Rust library...");
    display_output(build_rust_lib(
        project,
        lib_platform(platform),
        build_options,
    ))
    .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Built Rust library");
    Ok(())
}

async fn build_gtk4_packaging_artifacts(
    project: &Project,
    build_options: BuildOptions,
) -> Result<()> {
    let spinner = shell::spinner("Building GTK4 app...");
    display_output(build_gtk4(project, build_options)).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Built GTK4 app");
    Ok(())
}

async fn build_hydrolysis_packaging_artifacts(
    project: &Project,
    platform: TargetPlatform,
    build_options: BuildOptions,
) -> Result<()> {
    if platform == TargetPlatform::Web {
        return Ok(());
    }

    let spinner = shell::spinner("Building hydrolysis app...");
    display_output(build_hydrolysis(
        project,
        hydrolysis_platform(platform),
        build_options,
    ))
    .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Built hydrolysis app");
    Ok(())
}

async fn package_artifact(args: &Args, context: &PackagingContext) -> Result<()> {
    let spinner = shell::spinner("Packaging application...");
    let artifact = display_output(package_artifact_inner(args, context)).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Packaged at {}", artifact.path().display());
    Ok(())
}

async fn package_artifact_inner(args: &Args, context: &PackagingContext) -> Result<Artifact> {
    let package_options = PackageOptions::packaging(args.distribution, !args.release);
    match context.backend {
        TargetBackend::Android => {
            let abis: Vec<AndroidAbi> = args.arch.iter().map(|arch| arch.to_abi()).collect();
            AndroidPlatform::package_with_abis(&context.project, package_options, &abis).await
        }
        TargetBackend::Apple => {
            package_apple(
                &context.project,
                lib_platform(args.platform),
                package_options,
            )
            .await
        }
        TargetBackend::Gtk4 => package_gtk4(&context.project, package_options).await,
        TargetBackend::Hydrolysis => {
            package_hydrolysis(
                &context.project,
                hydrolysis_platform(args.platform),
                package_options,
            )
            .await
        }
    }
}

fn resolve_backend(platform: TargetPlatform, backend: TargetBackend) -> Result<TargetBackend> {
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
            | (
                TargetPlatform::Windows | TargetPlatform::Web,
                TargetBackend::Hydrolysis
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
             - Web: hydrolysis",
            backend,
            platform
        );
    }

    Ok(backend)
}

fn validate_arch_args(backend: TargetBackend, arch: &[AndroidArch]) -> Result<()> {
    if backend == TargetBackend::Android && arch.is_empty() {
        bail!(
            "Android backend requires --arch.\n\
             Examples:\n  \
             water package --platform android --backend android --arch arm64\n  \
             water package --platform android --backend android --arch arm64,x86_64"
        );
    }

    if backend != TargetBackend::Android && !arch.is_empty() {
        bail!("--arch is only valid when packaging Android backend");
    }

    Ok(())
}

async fn check_toolchain_for_backend(
    platform: TargetPlatform,
    backend: TargetBackend,
    arch: &[AndroidArch],
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
                | TargetPlatform::Web => {
                    bail!("Internal error: Apple backend is not supported on {platform:?}");
                }
            };
            toolchain_checks::check_apple(sdk).await?;
        }
        TargetBackend::Android => {
            if platform != TargetPlatform::Android {
                bail!("Internal error: Android backend is not supported on {platform:?}");
            }
            let required_abis = arch.iter().map(|arch| arch.to_abi()).collect::<Vec<_>>();
            toolchain_checks::check_android_build_or_package_for_abis(&required_abis).await?;
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
                && platform != TargetPlatform::Web
            {
                bail!("Internal error: hydrolysis backend is not supported on {platform:?}");
            }
            if platform == TargetPlatform::Web {
                toolchain_checks::check_web().await?;
            } else {
                toolchain_checks::check_hydrolysis().await?;
            }
        }
    }
    Ok(())
}

fn validate_desktop_backend_platform_on_host(
    platform: TargetPlatform,
    backend: TargetBackend,
) -> Result<()> {
    if platform == TargetPlatform::Web {
        return Ok(());
    }

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
        TargetPlatform::Web => panic!("web is not a native lib target"),
    }
}

const fn hydrolysis_platform(platform: TargetPlatform) -> LibTargetPlatform {
    match platform {
        TargetPlatform::Macos => LibTargetPlatform::MacOS,
        TargetPlatform::Linux => LibTargetPlatform::Linux,
        TargetPlatform::Windows => LibTargetPlatform::Windows,
        TargetPlatform::Web => LibTargetPlatform::Web,
        TargetPlatform::Ios | TargetPlatform::IosSimulator | TargetPlatform::Android => {
            panic!("unsupported hydrolysis platform")
        }
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
        TargetPlatform::Web => "Web",
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

#[cfg(test)]
mod tests {
    use super::{AndroidArch, TargetBackend, TargetPlatform, resolve_backend, validate_arch_args};

    #[test]
    fn rejects_empty_arch_for_android_backend() {
        assert!(validate_arch_args(TargetBackend::Android, &[]).is_err());
    }

    #[test]
    fn rejects_arch_for_non_android_backend() {
        let err = validate_arch_args(TargetBackend::Apple, &[AndroidArch::Arm64])
            .expect_err("non-android --arch should fail");
        assert!(err.to_string().contains("--arch is only valid"));
    }

    #[test]
    fn accepts_android_arch_values() {
        assert!(validate_arch_args(TargetBackend::Android, &[AndroidArch::Arm64]).is_ok());
    }

    #[test]
    fn resolve_backend_validates_explicit_backend() {
        assert_eq!(
            resolve_backend(TargetPlatform::Android, TargetBackend::Android)
                .expect("android backend"),
            TargetBackend::Android
        );
        assert_eq!(
            resolve_backend(TargetPlatform::Windows, TargetBackend::Hydrolysis)
                .expect("windows backend"),
            TargetBackend::Hydrolysis
        );
        assert!(resolve_backend(TargetPlatform::Windows, TargetBackend::Gtk4).is_err());
        assert_eq!(
            resolve_backend(TargetPlatform::Web, TargetBackend::Hydrolysis).expect("web backend"),
            TargetBackend::Hydrolysis
        );
    }
}
