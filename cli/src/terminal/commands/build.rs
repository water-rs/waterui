//! `water build` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use eyre::{Result, bail};
use target_lexicon::{Aarch64Architecture, Architecture};

use super::TargetBackend;
use crate::shell::Shell;
use crate::{error, header, line, success};
use waterui_cli::toolchain_checks;
use waterui_cli::{
    android::{
        embedded,
        platform::{AndroidAbi, AndroidPlatform},
    },
    apple::{platform::build_rust_lib, toolchain::AppleSdk},
    build::{BuildOptions, BuildProfile, BuiltTarget},
    gtk4::platform::build_gtk4,
    hydrolysis::platform::build_hydrolysis,
    platform::TargetPlatform as LibTargetPlatform,
    project::{ManagedBackends, Project},
    winui::platform::build_winui,
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
    /// ESP32-S3 (Xtensa); unsupported until #1601.
    Esp32s3,
    /// ESP32-C3 (RISC-V); unsupported until #1601.
    Esp32c3,
    /// ESP32-P4 (RISC-V, hardware FPU); unsupported until #1601.
    Esp32p4,
}

/// Target architecture for building.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TargetArch {
    /// ARM64 / `AArch64` (Apple Silicon, modern Android devices).
    Arm64,
    /// `x86_64` (Android emulators on Intel/AMD).
    X86_64,
    /// `ARMv7` (older 32-bit Android devices).
    Armv7,
    /// x86 (older 32-bit Android emulators).
    X86,
}

impl TargetArch {
    const fn architecture(self) -> Architecture {
        match self {
            Self::Arm64 => Architecture::Aarch64(Aarch64Architecture::Aarch64),
            Self::X86_64 => Architecture::X86_64,
            Self::Armv7 => Architecture::Arm(target_lexicon::ArmArchitecture::Armv7),
            Self::X86 => Architecture::X86_32(target_lexicon::X86_32Architecture::I686),
        }
    }
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

    /// Target architecture. Apple targets support only arm64; Android defaults to arm64.
    #[arg(short, long, value_enum)]
    arch: Option<TargetArch>,

    /// Build in release mode (optimized).
    #[arg(long)]
    release: bool,

    /// Build fully unoptimized, skipping the light optimization development
    /// builds apply by default to backends whose per-frame cost is high.
    #[arg(long, conflicts_with = "release")]
    debug: bool,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,

    /// Output directory to copy the built library to.
    /// Only valid for Apple/Android backends.
    #[arg(long)]
    output_dir: Option<PathBuf>,

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

struct BuildContext {
    project: Project,
    backend: TargetBackend,
    build_options: BuildOptions,
}

/// Run the build command.
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    let Some(context) = prepare_build_context(shell, &args).await? else {
        return Ok(());
    };
    print_build_header(
        shell,
        &context.project,
        args.platform,
        context.backend,
        args.release,
    );

    if context.project.manifest().package.embedded {
        // The embedded build's toolchain check and AAR compile futures cross
        // clippy's `large_futures` threshold (16 KiB) on Windows, so the future
        // is pinned on the heap instead of the caller's stack.
        return Box::pin(run_embedded_build(shell, &args, &context)).await;
    }

    check_build_toolchain(shell, args.platform, context.backend, args.arch).await?;
    let result = Box::pin(execute_build(shell, &args, &context)).await;

    handle_build_result(shell, result, args.output_dir)
}

/// `water build` on an embedded project produces the artifact the host
/// application consumes — an Android AAR or Apple Swift package.
///
/// The AAR carries every ABI unless `--arch` narrows the set, lands at
/// `target/package/` inside the project, and is published to `mavenLocal`
/// under `<bundle_identifier>:<crate_name>:<crate_version>` so the host's
/// Gradle build picks up every rerun (water-rs/cli#223).
async fn run_embedded_build(shell: &Shell, args: &Args, context: &BuildContext) -> Result<()> {
    if args.output_dir.is_some() {
        bail!(
            "--output-dir does not apply to embedded projects: artifacts land at target/package/"
        );
    }
    if context.backend == TargetBackend::Apple {
        return Box::pin(run_embedded_apple_build(shell, args, context)).await;
    }
    if args.platform != TargetPlatform::Android {
        bail!("embedded projects support the Apple and Android backends");
    }

    let abis: Vec<AndroidAbi> = args.arch.map_or_else(
        || embedded::ALL_ABIS.to_vec(),
        |arch| vec![android_abi(arch)],
    );

    let spinner = shell.spinner("Checking toolchain...");
    toolchain_checks::check_android_build_or_package_for_abis(
        &waterui_cli::toolchain::Host::current(),
        &abis,
    )
    .await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "Toolchain ready");

    let spinner = shell.spinner("Compiling...");
    let result = Box::pin(embedded::build_aar(
        &context.project.with_std_output(shell.is_interactive()),
        &context.build_options,
        &abis,
    ))
    .await;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    match result {
        Ok(artifact) => {
            success!(
                shell,
                "Embedded artifact at {}",
                artifact.aar_path.display()
            );
            success!(shell, "Published to mavenLocal as {}", artifact.coordinate);
            line!(shell, "In the host Gradle project, add:");
            line!(shell, "    mavenLocal() to repositories");
            line!(
                shell,
                "    implementation(\"{}\") to dependencies",
                artifact.coordinate
            );
            line!(
                shell,
                "then mount the WaterUI root with WaterUiEmbedding + WaterUiRootView:"
            );
            line!(shell, "    val waterui = WaterUiEmbedding(this)");
            line!(shell, "    setContentView(WaterUiRootView(this, waterui))");
            Ok(())
        }
        Err(err) => {
            error!(shell, "Build failed: {err}");
            Err(err)
        }
    }
}

async fn run_embedded_apple_build(
    shell: &Shell,
    args: &Args,
    context: &BuildContext,
) -> Result<()> {
    let architecture = args.arch.map(TargetArch::architecture);
    let spinner = shell.spinner("Building embedded Apple package...");
    let result = Box::pin(waterui_cli::apple::embedded::build_xcframework(
        &context.project.with_std_output(shell.is_interactive()),
        &context.build_options,
        architecture,
    ))
    .await;
    if let Some(progress) = spinner {
        progress.finish_and_clear();
    }
    let artifact = result?;
    success!(
        shell,
        "Embedded Swift package at {}",
        artifact.package_path.display()
    );
    line!(
        shell,
        "Add this local package to the native host and import WaterUI."
    );
    line!(
        shell,
        "Create one shared runtime on the main actor: let runtime = await WaterUIRuntime.create()"
    );
    line!(
        shell,
        "Mount WaterUIHost(runtime: runtime, resources: .module) or WaterUIHostController(runtime: runtime, resources: .module)."
    );
    Ok(())
}

async fn prepare_build_context(shell: &Shell, args: &Args) -> Result<Option<BuildContext>> {
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let backend = resolve_and_validate_backend(args)?;
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

    if backend.is_experimental()
        && !super::confirm_experimental_backend(shell, backend_name(backend), args.yes)?
    {
        return Ok(None);
    }

    let project = super::ensure_generated_backend(shell, project, backend).await?;
    let build_options = build_options(shell, args, backend).await;

    Ok(Some(BuildContext {
        project,
        backend,
        build_options,
    }))
}

fn resolve_and_validate_backend(args: &Args) -> Result<TargetBackend> {
    let backend = resolve_backend(args.platform, args.backend)?;
    backend
        .lib_backend()
        .validate_host_support(lib_platform(args.platform))?;
    validate_arch_args(args.platform, backend, args.arch)?;
    validate_output_dir_args(args.platform, backend, args.output_dir.as_ref())?;
    Ok(backend)
}

/// Resolve the Cargo profile `water build` builds under.
///
/// With no profile flag the backend's default development profile applies —
/// the same default `water run` uses, so a build's units are what a later
/// run reuses from the shared target directory rather than a variant the run
/// must recompile. `--debug` opts into a fully unoptimized build and
/// `--release` selects the release profile.
const fn build_profile(args: &Args, backend: TargetBackend) -> BuildProfile {
    if args.release {
        BuildProfile::Release
    } else if args.debug {
        BuildProfile::Debug
    } else {
        backend.lib_backend().default_development_profile()
    }
}

async fn build_options(shell: &Shell, args: &Args, backend: TargetBackend) -> BuildOptions {
    let profile = build_profile(args, backend);
    let mut build_options = args
        .output_dir
        .as_ref()
        .map_or_else(
            || BuildOptions::development(profile),
            |output_dir| BuildOptions::development(profile).with_output_dir(output_dir),
        )
        .with_progress(shell.build_progress());

    // `--release` is the shipped artifact, not a development unit: the shared
    // `waterui-dylib` runtime exists so `water run`/preview/mcp can reuse one
    // framework copy across sessions, and it makes the bin link the GPU stack
    // a second time (once inside the dylib through `waterui-internal/gpu`,
    // once statically through the backend). A production binary links the
    // whole stack once — statically — and stages as a single self-contained
    // file.
    if args.release {
        build_options = build_options.with_static_runtime();
    }

    if let Some(sccache_path) =
        super::detect_sccache_path(shell, &waterui_cli::toolchain::Host::current()).await
    {
        build_options = build_options.with_sccache(sccache_path);
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

async fn execute_build(shell: &Shell, args: &Args, context: &BuildContext) -> Result<BuiltTarget> {
    let spinner = shell.spinner("Compiling...");
    let result = Box::pin(async {
        // The project clone carries the interactive output policy into every
        // backend's build.
        let project = context.project.with_std_output(shell.is_interactive());
        match context.backend {
            TargetBackend::Apple => {
                build_for_apple(
                    &project,
                    args.platform,
                    args.arch,
                    context.build_options.clone(),
                )
                .await
            }
            TargetBackend::Android => {
                build_for_android(&project, args.arch, context.build_options.clone()).await
            }
            TargetBackend::Gtk4 => build_gtk4(&project, context.build_options.clone()).await,
            TargetBackend::Hydrolysis => {
                if args.platform == TargetPlatform::Android {
                    let abi = android_abi(args.arch.unwrap_or(TargetArch::Arm64));
                    waterui_cli::hydrolysis::android::build(
                        &project,
                        abi,
                        context.build_options.clone(),
                    )
                    .await
                } else {
                    build_hydrolysis(
                        &project,
                        lib_platform(args.platform),
                        context.build_options.clone(),
                    )
                    .await
                }
            }
            TargetBackend::WinUi => build_winui(&project, context.build_options.clone()).await,
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
    result: Result<BuiltTarget>,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    match result {
        Ok(built) => {
            success!(shell, "Build output at {}", built.profile_dir.display());
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
        TargetPlatform::Android | TargetPlatform::Linux | TargetPlatform::Windows => {
            TargetBackend::Hydrolysis
        }
        // No backend serves ESP32 targets yet: Dew is archived and
        // Hydrolysis's embedded host lands with #1601.
        TargetPlatform::Esp32s3 | TargetPlatform::Esp32c3 | TargetPlatform::Esp32p4 => {
            bail!("{}", super::ESP32_UNSUPPORTED)
        }
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
        ) | (
            TargetPlatform::Android,
            TargetBackend::Android | TargetBackend::Hydrolysis
        ) | (
            TargetPlatform::Linux,
            TargetBackend::Gtk4 | TargetBackend::Hydrolysis
        ) | (
            TargetPlatform::Windows,
            TargetBackend::Hydrolysis | TargetBackend::WinUi
        )
    );
    if !supported {
        bail!(
            "Backend {:?} does not support platform {:?}.\n\
             Valid combinations:\n  \
             - iOS/iOS Simulator: apple\n  \
             - Android: hydrolysis, android\n  \
             - macOS: apple, hydrolysis\n  \
             - Linux: gtk4, hydrolysis\n  \
             - Windows: hydrolysis, winui",
            backend,
            platform
        );
    }
    Ok(backend)
}

fn validate_arch_args(
    platform: TargetPlatform,
    backend: TargetBackend,
    arch: Option<TargetArch>,
) -> Result<()> {
    if backend == TargetBackend::Apple
        && let Some(arch) = arch
    {
        waterui_cli::apple::platform::validate_architecture(arch.architecture())?;
    }
    // Hydrolysis on Android takes --arch like the Android backend; on its
    // desktop platforms the triple is the host's.
    let arch_free_backend = matches!(backend, TargetBackend::Gtk4 | TargetBackend::WinUi)
        || (backend == TargetBackend::Hydrolysis && platform != TargetPlatform::Android);
    if arch_free_backend && arch.is_some() {
        bail!("--arch is not supported for gtk4/hydrolysis/winui backends");
    }
    Ok(())
}

fn validate_output_dir_args(
    platform: TargetPlatform,
    backend: TargetBackend,
    output_dir: Option<&PathBuf>,
) -> Result<()> {
    // Hydrolysis on Android stages the cdylib like the Android backend; the
    // generated Gradle project's `buildRust_*` tasks use it.
    let output_dir_supported = matches!(backend, TargetBackend::Apple | TargetBackend::Android)
        || (backend == TargetBackend::Hydrolysis && platform == TargetPlatform::Android);
    if output_dir.is_some() && !output_dir_supported {
        bail!("--output-dir is only supported for Apple/Android backends");
    }
    Ok(())
}

async fn check_toolchain_for_backend(
    host: &waterui_cli::toolchain::Host,
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
                | TargetPlatform::Esp32c3
                | TargetPlatform::Esp32p4 => {
                    bail!("Internal error: Apple backend is not supported on {platform:?}");
                }
            };
            toolchain_checks::check_apple(host, sdk).await?;
        }
        TargetBackend::Android => {
            if platform != TargetPlatform::Android {
                bail!("Internal error: Android backend is not supported on {platform:?}");
            }
            let requested_abi = android_abi(arch.unwrap_or(TargetArch::Arm64));
            toolchain_checks::check_android_build_or_package_for_abis(host, &[requested_abi])
                .await?;
        }
        TargetBackend::Gtk4 => {
            if platform != TargetPlatform::Linux {
                bail!("Internal error: GTK4 backend is not supported on {platform:?}");
            }
            toolchain_checks::check_gtk4(host).await?;
        }
        TargetBackend::Hydrolysis => {
            if platform == TargetPlatform::Android {
                let requested_abi = android_abi(arch.unwrap_or(TargetArch::Arm64));
                toolchain_checks::check_android_build_or_package_for_abis(host, &[requested_abi])
                    .await?;
            } else if platform != TargetPlatform::Macos
                && platform != TargetPlatform::Linux
                && platform != TargetPlatform::Windows
            {
                bail!("Internal error: hydrolysis backend is not supported on {platform:?}");
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

async fn build_for_apple(
    project: &Project,
    platform: TargetPlatform,
    arch: Option<TargetArch>,
    options: BuildOptions,
) -> Result<BuiltTarget> {
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
        (TargetPlatform::IosSimulator, None | Some(TargetArch::Arm64)) => {
            build_rust_lib(project, LibTargetPlatform::IOSSimulator, options).await
        }
        (TargetPlatform::IosSimulator, Some(target_arch)) => {
            bail!("iOS Simulator only supports arm64, not {:?}", target_arch);
        }
        (TargetPlatform::Macos, None | Some(TargetArch::Arm64)) => {
            build_rust_lib(project, LibTargetPlatform::MacOS, options).await
        }
        (TargetPlatform::Macos, Some(target_arch)) => {
            bail!("macOS only supports arm64, not {:?}", target_arch);
        }
        (
            TargetPlatform::Android
            | TargetPlatform::Linux
            | TargetPlatform::Windows
            | TargetPlatform::Esp32s3
            | TargetPlatform::Esp32c3
            | TargetPlatform::Esp32p4,
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
) -> Result<BuiltTarget> {
    let abi = android_abi(arch.unwrap_or(TargetArch::Arm64));
    AndroidPlatform::new(abi).build(project, options).await
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
        TargetPlatform::Esp32p4 => LibTargetPlatform::Esp32P4,
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
        TargetPlatform::Esp32p4 => "ESP32-P4",
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
        Args, TargetArch, TargetBackend, TargetPlatform, build_profile, resolve_backend,
        validate_arch_args, validate_output_dir_args,
    };
    use clap::Parser as _;
    use waterui_cli::build::BuildProfile;

    #[test]
    fn apple_rejects_intel_but_android_keeps_all_architectures() {
        for platform in [
            TargetPlatform::Macos,
            TargetPlatform::Ios,
            TargetPlatform::IosSimulator,
        ] {
            for arch in [TargetArch::X86_64, TargetArch::X86, TargetArch::Armv7] {
                let error =
                    validate_arch_args(platform, TargetBackend::Apple, Some(arch)).unwrap_err();
                assert!(error.to_string().contains("only support arm64"));
                assert!(
                    validate_arch_args(TargetPlatform::Android, TargetBackend::Android, Some(arch))
                        .is_ok()
                );
            }
            assert!(validate_arch_args(platform, TargetBackend::Apple, None).is_ok());
            assert!(
                validate_arch_args(platform, TargetBackend::Apple, Some(TargetArch::Arm64)).is_ok()
            );
        }
    }

    /// The build `Args` wrapped in a `Parser` so tests can exercise the real
    /// flag surface instead of constructing the clap struct field by field.
    #[derive(clap::Parser)]
    struct TestCli {
        #[command(flatten)]
        args: Args,
    }

    fn build_args(argv: &[&str]) -> Args {
        let mut full = vec!["water-build", "--platform", "windows"];
        full.extend_from_slice(argv);
        TestCli::try_parse_from(full)
            .expect("build args parse")
            .args
    }

    #[test]
    fn build_profile_defaults_to_backend_development_profile() {
        // Regression: `water build` used to always pick the declared dev
        // profile while `water run` built Hydrolysis with the Optimized
        // development profile. Sharing one target dir, the mismatch
        // re-fingerprinted every dependency unit — the run then cold-compiled
        // the whole graph (nightly "Fresh user / Windows" timeout).
        use clap::ValueEnum;
        let args = build_args(&[]);
        for backend in TargetBackend::value_variants() {
            assert_eq!(
                build_profile(&args, *backend),
                backend.lib_backend().default_development_profile(),
                "{backend:?} flag-free profile drifted from the backend default"
            );
        }
        assert_eq!(
            build_profile(&args, TargetBackend::Hydrolysis),
            BuildProfile::Optimized
        );
        assert_eq!(
            build_profile(&args, TargetBackend::Apple),
            BuildProfile::Debug
        );
    }

    #[test]
    fn build_profile_flags_override_the_default() {
        assert_eq!(
            build_profile(&build_args(&["--debug"]), TargetBackend::Hydrolysis),
            BuildProfile::Debug
        );
        assert_eq!(
            build_profile(&build_args(&["--release"]), TargetBackend::Hydrolysis),
            BuildProfile::Release
        );
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
    fn resolve_backend_defaults_match_platforms() {
        assert_eq!(
            resolve_backend(TargetPlatform::Ios, None).expect("ios backend"),
            TargetBackend::Apple
        );
        assert_eq!(
            resolve_backend(TargetPlatform::Android, None).expect("android backend"),
            TargetBackend::Hydrolysis
        );
        assert_eq!(
            resolve_backend(TargetPlatform::Linux, None).expect("linux backend"),
            TargetBackend::Hydrolysis
        );
        assert_eq!(
            resolve_backend(TargetPlatform::Windows, None).expect("windows backend"),
            TargetBackend::Hydrolysis
        );
    }

    #[test]
    fn output_dir_rejected_for_desktop_backends() {
        let output = Some(&std::path::PathBuf::from("/tmp/out"));
        assert!(
            validate_output_dir_args(TargetPlatform::Linux, TargetBackend::Gtk4, output).is_err()
        );
        assert!(
            validate_output_dir_args(TargetPlatform::Linux, TargetBackend::Hydrolysis, output)
                .is_err()
        );
        assert!(
            validate_output_dir_args(TargetPlatform::Android, TargetBackend::Hydrolysis, output)
                .is_ok()
        );
        assert!(
            validate_output_dir_args(TargetPlatform::Macos, TargetBackend::Apple, output).is_ok()
        );
    }
}
