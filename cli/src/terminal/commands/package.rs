//! `water package` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use eyre::{Result, bail};

use crate::shell::Shell;
use crate::{header, success};
use waterui_cli::toolchain_checks;
use waterui_cli::{
    android::platform::{AndroidAbi, AndroidPlatform},
    apple::{
        platform::{build_rust_lib, package_apple, stage_packaged_host_library},
        toolchain::AppleSdk,
    },
    build::{BuildOptions, BuildProfile, BuiltTarget, stage_dxc_runtime},
    device::Artifact,
    gtk4::platform::{build_gtk4, package_gtk4},
    hydrolysis::{
        android::{self as hydrolysis_android, HydrolysisAndroidPainter},
        platform::{build_hydrolysis, package_hydrolysis},
    },
    package_output::place_in_project,
    platform::{
        DeviceSigning, PackageAudience, PackageOptions, TargetPlatform as LibTargetPlatform,
    },
    project::{ManagedBackends, Project},
    winui::platform::{build_winui, package_winui},
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
    /// `WinUI` backend.
    #[value(name = "winui")]
    WinUi,
}

impl TargetBackend {
    /// Whether the backend is experimental — shipped without full testing
    /// ahead of milestone releases — so selecting it asks for confirmation.
    const fn is_experimental(self) -> bool {
        matches!(self, Self::Gtk4 | Self::WinUi)
    }

    /// The shared command-line backend this packaging backend is.
    const fn cli_backend(self) -> super::TargetBackend {
        match self {
            Self::Apple => super::TargetBackend::Apple,
            Self::Android => super::TargetBackend::Android,
            Self::Gtk4 => super::TargetBackend::Gtk4,
            Self::Hydrolysis => super::TargetBackend::Hydrolysis,
            Self::WinUi => super::TargetBackend::WinUi,
        }
    }
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

    /// Android painter the Hydrolysis host draws with (gpu, hwui).
    /// Only valid with `--platform android --backend hydrolysis`; the
    /// `[hydrolysis] painter` table in `Water.toml` is the project
    /// default when omitted.
    #[arg(long, value_enum)]
    painter: Option<HydrolysisAndroidPainter>,

    #[command(flatten)]
    profile: ProfileArgs,

    /// Package for store distribution (App Store, Play Store).
    #[arg(long)]
    distribution: bool,

    /// Leave a release build unsigned, for a host with no signing identity
    /// (CI, a build VM). Applies to iOS device builds with the Apple backend
    /// and Android builds with the Android or Hydrolysis backend. Linkage and
    /// profile are those of the signed build; sign the artifact (`codesign`,
    /// `apksigner`) before installing it. On Android it combines with
    /// `--distribution` to produce an unsigned AAB; on iOS it does not.
    #[arg(long)]
    unsigned: bool,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,

    /// Target architectures for Android (comma-separated).
    /// Examples: --arch arm64, --arch `arm64,x86-64`
    /// Required when packaging Android backend.
    #[arg(long, value_enum, value_delimiter = ',')]
    arch: Vec<AndroidArch>,

    /// Skip the confirmation prompt required by experimental backends
    /// (needed in non-interactive environments).
    #[arg(short = 'y', long)]
    yes: bool,
}

impl Args {
    /// The project directory this command works on.
    pub(crate) fn project_dir(&self) -> &std::path::Path {
        &self.path
    }
}

/// The build profile flags: at most one of them, and `release` when neither is
/// given — `water package` builds what users ship unless asked otherwise.
#[derive(ClapArgs, Debug)]
#[group(multiple = false)]
struct ProfileArgs {
    /// Build the shipped `release` profile. This is the default; the flag
    /// states it explicitly.
    #[arg(long)]
    release: bool,

    /// Build the unoptimized `dev` profile instead of the shipped `release`
    /// profile.
    #[arg(long, conflicts_with = "distribution")]
    debug: bool,
}

impl ProfileArgs {
    const fn build_profile(&self) -> BuildProfile {
        if self.release || !self.debug {
            BuildProfile::Release
        } else {
            BuildProfile::Debug
        }
    }
}

impl Args {
    /// The profile the package builds.
    const fn profile(&self) -> BuildProfile {
        self.profile.build_profile()
    }
}

struct PackagingContext {
    project: Project,
    backend: TargetBackend,
    build_options: BuildOptions,
    /// The final package options the Gradle/Xcode step runs with, resolved
    /// alongside the signing plan so the plan binds to exactly these values.
    package_options: PackageOptions,
    /// The release-signing plan an Android-platform package resolved before
    /// the Rust builds — bound to this project and `package_options`.
    /// `None` on every other platform.
    prepared_signing: Option<waterui_cli::android::signing::PreparedSigning>,
}

/// Run the package command.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    // The packaging context carries the opened project, the resolved backend and
    // the build options; on Windows that future crosses clippy's `large_futures`
    // threshold (16 KiB), so it is pinned on the heap instead of the caller's stack.
    let Some(context) = Box::pin(prepare_packaging_context(shell, &args)).await? else {
        return Ok(());
    };
    print_packaging_header(
        shell,
        &context.project,
        args.platform,
        context.backend,
        args.profile(),
        args.distribution,
    );
    check_packaging_toolchain(shell, args.platform, context.backend, &args.arch).await?;
    // The per-backend artifact builds cross clippy's `large_futures` threshold
    // (16 KiB) on Windows, so the future is pinned on the heap.
    let built = Box::pin(build_packaging_artifacts(shell, &args, &context)).await?;
    package_artifact(shell, &args, &context, built.as_ref()).await
}

async fn prepare_packaging_context(shell: &Shell, args: &Args) -> Result<Option<PackagingContext>> {
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let backend = resolve_backend(args.platform, args.backend)?;
    // Hydrolysis on Android opens no managed backend — the old widget-FFI
    // backend is not its runtime; the Hydrolysis launcher crate
    // `ensure_generated_backend` produces is.
    let managed_backends =
        if (args.platform, backend) == (TargetPlatform::Android, TargetBackend::Hydrolysis) {
            ManagedBackends::NONE
        } else {
            ManagedBackends::for_platform(lib_platform(args.platform))
        };
    let project = Project::open(
        &waterui_cli::toolchain::Host::current(),
        &project_path,
        managed_backends,
    )
    .await?;
    if project.manifest().package.embedded {
        bail!(
            "`water package` does not apply to embedded projects: `water build` already produces the host-consumable artifact"
        );
    }

    validate_arch_args(args.platform, backend, &args.arch)?;
    validate_unsigned_args(
        args.platform,
        backend,
        args.profile(),
        args.unsigned,
        args.distribution,
    )?;
    backend
        .cli_backend()
        .lib_backend()
        .validate_host_support(lib_platform(args.platform))?;
    if args.painter.is_some()
        && !(args.platform == TargetPlatform::Android && backend == TargetBackend::Hydrolysis)
    {
        bail!("--painter only applies to `--platform android --backend hydrolysis`");
    }

    // Resolve the release-signing plan once, before the Rust builds: a
    // release package without [signing.android] can only produce artifacts
    // nothing installs or accepts. The plan is bound to this project and to
    // the final package options, and the Gradle step re-proves that binding
    // instead of re-probing the manifest and the environment. `[signing.
    // android]` is an Android platform contract, so both Android backends
    // resolve it here.
    let package_options = PackageOptions::packaging(
        if args.distribution {
            PackageAudience::Distribution
        } else {
            PackageAudience::Development
        },
        args.profile().is_development(),
    )
    .with_device_signing(if args.unsigned {
        DeviceSigning::Unsigned
    } else {
        DeviceSigning::Automatic
    })
    .with_progress(shell.build_progress());
    let prepared_signing = (lib_platform(args.platform) == LibTargetPlatform::Android)
        .then(|| {
            waterui_cli::android::signing::PreparedSigning::resolve(&project, &package_options)
        })
        .transpose()?;

    if backend.is_experimental()
        && !super::confirm_experimental_backend(shell, backend_name(backend), args.yes)?
    {
        return Ok(None);
    }
    let project = super::ensure_generated_backend(shell, project, backend.cli_backend()).await?;

    let mut build_options =
        BuildOptions::packaging(args.profile()).with_progress(shell.build_progress());
    if let Some(sccache_path) =
        super::detect_sccache_path(shell, &waterui_cli::toolchain::Host::current()).await
    {
        build_options = build_options.with_sccache(sccache_path);
    }

    Ok(Some(PackagingContext {
        project,
        backend,
        build_options,
        package_options,
        prepared_signing,
    }))
}

fn print_packaging_header(
    shell: &Shell,
    project: &Project,
    platform: TargetPlatform,
    backend: TargetBackend,
    profile: BuildProfile,
    distribution: bool,
) {
    let mode = if profile.is_release() {
        "release"
    } else {
        "debug"
    };
    let dist = if distribution { " (distribution)" } else { "" };
    header!(
        shell,
        "Packaging {} for {} via {} ({}){}",
        project.crate_name(),
        platform_name(platform),
        backend_name(backend),
        mode,
        dist
    );
}

async fn check_packaging_toolchain(
    shell: &Shell,
    platform: TargetPlatform,
    backend: TargetBackend,
    arch: &[AndroidArch],
) -> Result<()> {
    let spinner = shell.spinner("Checking toolchain...");
    check_toolchain_for_backend(
        &waterui_cli::toolchain::Host::current(),
        platform,
        backend,
        arch,
    )
    .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "Toolchain ready");
    Ok(())
}

async fn build_packaging_artifacts(
    shell: &Shell,
    args: &Args,
    context: &PackagingContext,
) -> Result<Option<BuiltTarget>> {
    match context.backend {
        TargetBackend::Android => {
            // The per-backend artifact builds cross clippy's `large_futures`
            // threshold (16 KiB) on Windows, so the future is pinned on the heap.
            Box::pin(build_android_packaging_artifacts(
                shell,
                &context.project,
                &args.arch,
                context.build_options.clone(),
            ))
            .await
        }
        TargetBackend::Apple => {
            Box::pin(build_apple_packaging_artifacts(
                shell,
                &context.project,
                args.platform,
                context.build_options.clone(),
            ))
            .await
        }
        TargetBackend::Gtk4 => {
            build_gtk4_packaging_artifacts(shell, &context.project, context.build_options.clone())
                .await
        }
        TargetBackend::Hydrolysis => {
            Box::pin(build_hydrolysis_packaging_artifacts(
                shell,
                &context.project,
                args.platform,
                &args.arch,
                context.build_options.clone(),
            ))
            .await
        }
        TargetBackend::WinUi => {
            build_winui_packaging_artifacts(shell, &context.project, context.build_options.clone())
                .await
        }
    }
}

async fn build_android_packaging_artifacts(
    shell: &Shell,
    project: &Project,
    arch: &[AndroidArch],
    build_options: BuildOptions,
) -> Result<Option<BuiltTarget>> {
    let mut built = None;
    AndroidPlatform::clean_jni_libs(project).await?;
    for arch in arch {
        let abi = arch.to_abi();
        let spinner = shell.spinner(format!("Building Rust library ({})...", abi.as_str()));
        // The Android build future crosses clippy's `large_futures` threshold
        // (16 KiB) on Windows, so it is pinned on the heap.
        let target = Box::pin(
            shell.display_output(AndroidPlatform::new(abi).build(project, build_options.clone())),
        )
        .await?;
        built = Some(target);
        if let Some(pb) = spinner {
            pb.finish_and_clear();
        }
        success!(shell, "Built for {}", abi.as_str());
    }
    Ok(built)
}

async fn build_apple_packaging_artifacts(
    shell: &Shell,
    project: &Project,
    platform: TargetPlatform,
    build_options: BuildOptions,
) -> Result<Option<BuiltTarget>> {
    let spinner = shell.spinner("Building Rust library...");
    let built = shell
        .display_output(Box::pin(build_rust_lib(
            project,
            lib_platform(platform),
            build_options,
        )))
        .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "Built Rust library");
    Ok(Some(built))
}

async fn build_gtk4_packaging_artifacts(
    shell: &Shell,
    project: &Project,
    build_options: BuildOptions,
) -> Result<Option<BuiltTarget>> {
    let spinner = shell.spinner("Building GTK4 app...");
    let built = shell
        .display_output(build_gtk4(project, build_options))
        .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "Built GTK4 app");
    Ok(Some(built))
}

async fn build_winui_packaging_artifacts(
    shell: &Shell,
    project: &Project,
    build_options: BuildOptions,
) -> Result<Option<BuiltTarget>> {
    let spinner = shell.spinner("Building WinUI app...");
    let built = shell
        .display_output(build_winui(project, build_options))
        .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "Built WinUI app");
    Ok(Some(built))
}

async fn build_hydrolysis_packaging_artifacts(
    shell: &Shell,
    project: &Project,
    platform: TargetPlatform,
    arch: &[AndroidArch],
    build_options: BuildOptions,
) -> Result<Option<BuiltTarget>> {
    if platform == TargetPlatform::Web {
        return Ok(None);
    }

    if platform == TargetPlatform::Android {
        let mut built = None;
        hydrolysis_android::clean_jni_libs(project).await?;
        for arch in arch {
            let abi = arch.to_abi();
            let spinner = shell.spinner(format!("Building Rust library ({})...", abi.as_str()));
            let target = shell
                .display_output(hydrolysis_android::build(
                    project,
                    &waterui_cli::toolchain::Host::current(),
                    abi,
                    build_options.clone(),
                ))
                .await?;
            built = Some(target);
            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }
            success!(shell, "Built for {}", abi.as_str());
        }
        return Ok(built);
    }

    let spinner = shell.spinner("Building hydrolysis app...");
    let built = shell
        .display_output(Box::pin(build_hydrolysis(
            project,
            hydrolysis_platform(platform),
            build_options,
        )))
        .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "Built hydrolysis app");
    Ok(Some(built))
}

async fn package_artifact(
    shell: &Shell,
    args: &Args,
    context: &PackagingContext,
    built: Option<&BuiltTarget>,
) -> Result<()> {
    let spinner = shell.spinner("Packaging application...");
    let artifact = shell
        .display_output(package_artifact_inner(args, context, built))
        .await?;
    let artifact = place_in_project(&context.project, artifact).await?;
    // Consumers read the host library beside the `.app` this command reports,
    // so it stages against the placed path, which only exists after the move.
    if context.backend == TargetBackend::Apple
        && let Some(built) = built
        && let Some(dir) = artifact.path().parent()
    {
        stage_packaged_host_library(built, dir).await?;
    }
    // A packaged Windows hydrolysis binary is statically linked, but wgpu's
    // DirectX 12 backend still `LoadLibrary`s `dxcompiler.dll` and `dxil.dll`
    // by name at run time; the pair has to ship beside the artifact.
    if cfg!(windows)
        && context.backend == TargetBackend::Hydrolysis
        && args.platform == TargetPlatform::Windows
    {
        let destination = artifact.path().parent().ok_or_else(|| {
            eyre::eyre!(
                "packaged artifact {} has no parent directory",
                artifact.path().display()
            )
        })?;
        stage_dxc_runtime(context.project.host(), destination).await?;
    }
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "Packaged at {}", artifact.path().display());
    Ok(())
}

async fn package_artifact_inner(
    args: &Args,
    context: &PackagingContext,
    built: Option<&BuiltTarget>,
) -> Result<Artifact> {
    let package_options = context.package_options.clone();
    match context.backend {
        TargetBackend::Android => {
            let abis: Vec<AndroidAbi> = args.arch.iter().map(|arch| arch.to_abi()).collect();
            let built = built.ok_or_else(|| {
                eyre::eyre!("Internal error: Android packaging has no build result")
            })?;
            AndroidPlatform::package_with_abis(
                &context.project,
                package_options,
                &abis,
                built,
                context.prepared_signing.as_ref().ok_or_else(|| {
                    eyre::eyre!("Internal error: Android packaging has no signing plan")
                })?,
            )
            .await
        }
        TargetBackend::Apple => {
            let built = built.ok_or_else(|| {
                eyre::eyre!("Internal error: Apple packaging has no build result")
            })?;
            package_apple(
                &context.project,
                lib_platform(args.platform),
                package_options,
                built,
            )
            .await
        }
        TargetBackend::Gtk4 => {
            package_gtk4(
                &context.project,
                package_options,
                built.ok_or_else(|| {
                    eyre::eyre!("Internal error: GTK4 packaging has no build result")
                })?,
            )
            .await
        }
        TargetBackend::WinUi => {
            package_winui(
                &context.project,
                package_options,
                built.ok_or_else(|| {
                    eyre::eyre!("Internal error: WinUI packaging has no build result")
                })?,
            )
            .await
        }
        TargetBackend::Hydrolysis => {
            if args.platform == TargetPlatform::Android {
                let abis: Vec<AndroidAbi> = args.arch.iter().map(|arch| arch.to_abi()).collect();
                let painter = hydrolysis_android::resolve_painter(&context.project, args.painter);
                hydrolysis_android::package_with_abis(
                    &context.project,
                    &waterui_cli::toolchain::Host::current(),
                    painter,
                    &package_options,
                    &abis,
                    built.ok_or_else(|| {
                        eyre::eyre!(
                            "Internal error: Hydrolysis Android packaging has no build result"
                        )
                    })?,
                    context.prepared_signing.as_ref().ok_or_else(|| {
                        eyre::eyre!("Internal error: Android packaging has no signing plan")
                    })?,
                )
                .await
            } else {
                package_hydrolysis(
                    &context.project,
                    hydrolysis_platform(args.platform),
                    package_options,
                    built,
                )
                .await
            }
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
        ) | (
            TargetPlatform::Android,
            TargetBackend::Android | TargetBackend::Hydrolysis
        ) | (
            TargetPlatform::Linux,
            TargetBackend::Gtk4 | TargetBackend::Hydrolysis
        ) | (
            TargetPlatform::Windows,
            TargetBackend::Hydrolysis | TargetBackend::WinUi
        ) | (TargetPlatform::Web, TargetBackend::Hydrolysis)
    );

    if !supported {
        bail!(
            "Backend {:?} does not support platform {:?}.\n\
             Valid combinations:\n  \
             - iOS/iOS Simulator: apple\n  \
             - Android: hydrolysis, android\n  \
             - macOS: apple, hydrolysis\n  \
             - Linux: gtk4, hydrolysis\n  \
             - Windows: hydrolysis, winui\n  \
             - Web: hydrolysis",
            backend,
            platform
        );
    }

    Ok(backend)
}

fn validate_unsigned_args(
    platform: TargetPlatform,
    backend: TargetBackend,
    profile: BuildProfile,
    unsigned: bool,
    distribution: bool,
) -> Result<()> {
    let android = platform == TargetPlatform::Android;
    let applies = (platform == TargetPlatform::Ios && backend == TargetBackend::Apple)
        || (android && matches!(backend, TargetBackend::Android | TargetBackend::Hydrolysis));
    if unsigned && !applies {
        bail!(
            "--unsigned only applies to an iOS device build with the Apple backend or an \
             Android build with the Android or Hydrolysis backend; simulator and desktop \
             packages are not signed per device"
        );
    }
    // Debug+unsigned is only a problem on Android, where --unsigned would be
    // silently ignored: an Android debug package always signs with the debug
    // keystore. iOS's credential-free debug package path relies on the
    // combination, so it stays valid there.
    if unsigned && profile.is_development() && android {
        bail!(
            "--unsigned applies to release packages only; a debug build signs \
             with the debug signing identity"
        );
    }
    // The App Store rejects unsigned bundles, so an unsigned iOS
    // distribution package cannot exist; Play accepts an unsigned AAB the
    // uploader signs themselves, so Android permits the combination.
    if unsigned && distribution && platform == TargetPlatform::Ios {
        bail!(
            "--unsigned cannot produce an iOS distribution package; the App Store requires a signed bundle"
        );
    }
    Ok(())
}

fn validate_arch_args(
    platform: TargetPlatform,
    backend: TargetBackend,
    arch: &[AndroidArch],
) -> Result<()> {
    let packages_android = backend == TargetBackend::Android
        || (platform == TargetPlatform::Android && backend == TargetBackend::Hydrolysis);

    if packages_android && arch.is_empty() {
        let backend_arg = format!("{backend:?}").to_lowercase();
        bail!(
            "Packaging for Android requires --arch.\n\
             Examples:\n  \
             water package --platform android --backend {backend_arg} --arch arm64\n  \
             water package --platform android --backend {backend_arg} --arch arm64,x86-64"
        );
    }

    if !packages_android && !arch.is_empty() {
        bail!("--arch is only valid when packaging for Android");
    }

    Ok(())
}

async fn check_toolchain_for_backend(
    host: &waterui_cli::toolchain::Host,
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
            toolchain_checks::check_apple(host, sdk).await?;
        }
        TargetBackend::Android => {
            if platform != TargetPlatform::Android {
                bail!("Internal error: Android backend is not supported on {platform:?}");
            }
            let required_abis = arch.iter().map(|arch| arch.to_abi()).collect::<Vec<_>>();
            toolchain_checks::check_android_build_or_package_for_abis(host, &required_abis).await?;
        }
        TargetBackend::Gtk4 => {
            if platform != TargetPlatform::Linux {
                bail!("Internal error: GTK4 backend is not supported on {platform:?}");
            }
            toolchain_checks::check_gtk4(host).await?;
        }
        TargetBackend::Hydrolysis => {
            if platform != TargetPlatform::Macos
                && platform != TargetPlatform::Linux
                && platform != TargetPlatform::Windows
                && platform != TargetPlatform::Web
                && platform != TargetPlatform::Android
            {
                bail!("Internal error: hydrolysis backend is not supported on {platform:?}");
            }
            if platform == TargetPlatform::Android {
                let required_abis = arch.iter().map(|arch| arch.to_abi()).collect::<Vec<_>>();
                toolchain_checks::check_android_build_or_package_for_abis(host, &required_abis)
                    .await?;
            } else if platform == TargetPlatform::Web {
                toolchain_checks::check_web(host).await?;
            } else {
                toolchain_checks::check_hydrolysis(host).await?;
            }
        }
        TargetBackend::WinUi => {
            if platform != TargetPlatform::Windows {
                bail!("Internal error: WinUI backend is not supported on {platform:?}");
            }
            toolchain_checks::check_winui(host).await?;
        }
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
        TargetPlatform::Web => LibTargetPlatform::Web,
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
        TargetBackend::WinUi => "WinUI",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AndroidArch, BuildProfile, TargetBackend, TargetPlatform, resolve_backend,
        validate_arch_args, validate_unsigned_args,
    };

    #[test]
    fn unsigned_args_follow_the_platform_signing_contract() {
        let v = validate_unsigned_args;
        let (ios, android, macos) = (
            TargetPlatform::Ios,
            TargetPlatform::Android,
            TargetPlatform::Macos,
        );
        let (apple, android_be, hydrolysis, gtk4) = (
            TargetBackend::Apple,
            TargetBackend::Android,
            TargetBackend::Hydrolysis,
            TargetBackend::Gtk4,
        );
        let (release, debug) = (BuildProfile::Release, BuildProfile::Debug);

        // iOS: release unsigned is the credential-free path — including debug,
        // which never needed an identity — but an unsigned App Store bundle
        // cannot exist.
        assert!(v(ios, apple, release, true, false).is_ok());
        assert!(v(ios, apple, debug, true, false).is_ok());
        assert!(v(ios, apple, release, true, true).is_err());

        // Android, both backends: unsigned is valid for release APK and AAB
        // (distribution), never for debug — the flag would be ignored since a
        // debug build always signs with the debug keystore.
        for backend in [android_be, hydrolysis] {
            assert!(
                v(android, backend, release, true, false).is_ok(),
                "{backend:?}"
            );
            assert!(
                v(android, backend, release, true, true).is_ok(),
                "{backend:?}"
            );
            assert!(
                v(android, backend, debug, true, false).is_err(),
                "{backend:?}"
            );
        }

        // Everywhere else --unsigned means nothing.
        assert!(v(macos, apple, release, true, false).is_err());
        assert!(v(ios, hydrolysis, release, true, false).is_err());
        assert!(v(macos, gtk4, release, true, false).is_err());
        // No --unsigned: every combination validates.
        for (platform, backend) in [
            (ios, apple),
            (android, android_be),
            (android, hydrolysis),
            (macos, gtk4),
        ] {
            assert!(v(platform, backend, release, false, false).is_ok());
            assert!(v(platform, backend, debug, false, false).is_ok());
        }
    }

    #[test]
    fn only_gtk4_and_winui_are_experimental() {
        use clap::ValueEnum;
        for backend in TargetBackend::value_variants() {
            assert_eq!(
                backend.is_experimental(),
                matches!(backend, TargetBackend::Gtk4 | TargetBackend::WinUi),
                "{backend:?} experimental flag drifted"
            );
        }
    }

    #[test]
    fn rejects_empty_arch_for_android_backend() {
        assert!(validate_arch_args(TargetPlatform::Android, TargetBackend::Android, &[]).is_err());
        assert!(
            validate_arch_args(TargetPlatform::Android, TargetBackend::Hydrolysis, &[]).is_err()
        );
    }

    #[test]
    fn rejects_arch_for_non_android_backend() {
        let err = validate_arch_args(
            TargetPlatform::Macos,
            TargetBackend::Apple,
            &[AndroidArch::Arm64],
        )
        .expect_err("non-android --arch should fail");
        assert!(err.to_string().contains("--arch is only valid"));
        // Hydrolysis carries --arch only on Android.
        assert!(
            validate_arch_args(
                TargetPlatform::Linux,
                TargetBackend::Hydrolysis,
                &[AndroidArch::Arm64]
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_android_arch_values() {
        assert!(
            validate_arch_args(
                TargetPlatform::Android,
                TargetBackend::Android,
                &[AndroidArch::Arm64]
            )
            .is_ok()
        );
        assert!(
            validate_arch_args(
                TargetPlatform::Android,
                TargetBackend::Hydrolysis,
                &[AndroidArch::Arm64]
            )
            .is_ok()
        );
    }

    #[test]
    fn resolve_backend_validates_explicit_backend() {
        assert_eq!(
            resolve_backend(TargetPlatform::Android, TargetBackend::Android)
                .expect("android backend"),
            TargetBackend::Android
        );
        assert_eq!(
            resolve_backend(TargetPlatform::Android, TargetBackend::Hydrolysis)
                .expect("hydrolysis on android backend"),
            TargetBackend::Hydrolysis
        );
        assert_eq!(
            resolve_backend(TargetPlatform::Windows, TargetBackend::Hydrolysis)
                .expect("windows backend"),
            TargetBackend::Hydrolysis
        );
        assert!(resolve_backend(TargetPlatform::Windows, TargetBackend::Gtk4).is_err());
        assert_eq!(
            resolve_backend(TargetPlatform::Windows, TargetBackend::WinUi)
                .expect("windows winui backend"),
            TargetBackend::WinUi
        );
        assert!(resolve_backend(TargetPlatform::Linux, TargetBackend::WinUi).is_err());
        assert_eq!(
            resolve_backend(TargetPlatform::Web, TargetBackend::Hydrolysis).expect("web backend"),
            TargetBackend::Hydrolysis
        );
    }
}
