//! `water run` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use eyre::{Context as _, Result, bail};
use futures_util::StreamExt;

#[cfg(target_os = "macos")]
use jiff::Timestamp;
#[cfg(target_os = "macos")]
use waterui_cli::device::Crash;

use super::{TargetBackend, detect_sccache_path};
use crate::shell::Shell;
use crate::{error, header, line, note, success, warn};
use waterui_cli::toolchain_checks;
use waterui_cli::{
    android::{
        device::{AndroidDevice, AndroidEmulator},
        platform::AndroidPlatform,
    },
    apple::{
        device::AppleSimulator,
        physical::ApplePhysicalDevice,
        platform::{build_rust_lib, package_apple},
        toolchain::AppleSdk,
    },
    build::{BuildOptions, BuildProfile, BuildProgress},
    device::{Artifact, CrashCause, Device, DeviceEvent, Local, LogLevel, RunOptions, Running},
    esp32::platform::run_esp32,
    gtk4::platform::{build_gtk4, package_gtk4},
    hydrolysis::{
        android::{self as hydrolysis_android, HydrolysisAndroidPainter},
        platform::{
            HydrolysisWebDevServer, build_hydrolysis, package_hydrolysis,
            prepare_hydrolysis_web_dev_site,
        },
    },
    platform::{PackageOptions, TargetPlatform as LibTargetPlatform},
    project::{ManagedBackends, Project},
    web,
    winui::platform::{build_winui, package_winui},
};

#[cfg(target_os = "macos")]
use waterui_cli::debug;

#[cfg(target_os = "macos")]
struct CrashReportContext {
    started_at: Timestamp,
    device_identifier: String,
    bundle_id: String,
    process_name: String,
}

#[cfg(target_os = "macos")]
impl CrashReportContext {
    fn try_new(
        project: &Project,
        platform: TargetPlatform,
        backend: TargetBackend,
    ) -> Result<Option<Self>> {
        if platform != TargetPlatform::Macos || backend != TargetBackend::Apple {
            return Ok(None);
        }

        let device_identifier =
            whoami::hostname().map_err(|e| eyre::eyre!("Failed to determine hostname: {e}"))?;

        // A crash report is filed under the executable's name, which is the
        // product name — the same one the bundle is built under.
        let process_name = waterui_cli::apple::backend::apple_product_name(project)?.to_string();

        Ok(Some(Self {
            started_at: Timestamp::now(),
            device_identifier,
            bundle_id: project.bundle_identifier().to_string(),
            process_name,
        }))
    }

    fn refresh_start(&mut self) {
        self.started_at = Timestamp::now();
    }
}

#[cfg(target_os = "macos")]
async fn find_latest_ips_report(
    host: &waterui_cli::toolchain::Host,
    ctx: &CrashReportContext,
) -> Option<debug::CrashReport> {
    debug::find_macos_ips_crash_report_since(
        host,
        "macOS",
        &ctx.device_identifier,
        &ctx.bundle_id,
        &ctx.process_name,
        None,
        ctx.started_at,
    )
    .await
}

struct RunContext {
    project: Project,
    platform: TargetPlatform,
    backend: TargetBackend,
}

struct DeviceSelection {
    device: SelectedDevice,
    needs_launch: bool,
}

struct BuildPlan {
    lib_platform: LibTargetPlatform,
    android_abi: Option<waterui_cli::android::platform::AndroidAbi>,
}

/// Target platform for running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TargetPlatform {
    /// iOS Simulator.
    Ios,
    /// Android.
    Android,
    /// macOS (current machine).
    Macos,
    /// Linux (native desktop).
    Linux,
    /// Windows (native desktop).
    Windows,
    /// Web (WASM + WebGPU in browser).
    Web,
    /// ESP32-S3 board or QEMU (Dew firmware, Xtensa).
    Esp32s3,
    /// ESP32-C3 board or QEMU (Dew firmware, RISC-V).
    Esp32c3,
    /// ESP32-P4 board (Dew firmware, RISC-V with FPU).
    Esp32p4,
}

impl TargetPlatform {
    /// The ESP32 chip a platform selects, if it is an ESP32 platform.
    const fn esp32_chip(self) -> Option<waterui_cli::esp32::chip::Esp32Chip> {
        use waterui_cli::esp32::chip::Esp32Chip;
        match self {
            Self::Esp32s3 => Some(Esp32Chip::Esp32S3),
            Self::Esp32c3 => Some(Esp32Chip::Esp32C3),
            Self::Esp32p4 => Some(Esp32Chip::Esp32P4),
            _ => None,
        }
    }

    /// The desktop OS this platform runs natively on the host, if it is one.
    ///
    /// Device, web and embedded platforms carry their own target triples;
    /// the desktop platforms resolve `Triple::host()` and only make sense on
    /// the OS they name.
    const fn desktop_os(self) -> Option<&'static str> {
        match self {
            Self::Macos => Some("macos"),
            Self::Linux => Some("linux"),
            Self::Windows => Some("windows"),
            _ => None,
        }
    }
}

/// Arguments for the run command.
#[derive(ClapArgs, Debug)]
// CLI flag structs collect booleans by nature; each flag is a documented
// `--flag`, not structural state.
#[expect(
    clippy::struct_excessive_bools,
    reason = "every boolean is a documented --flag, not structural state — the shape follows the CLI surface"
)]
pub struct Args {
    /// Target platform to run on.
    /// Defaults to the host platform when omitted.
    #[arg(short, long, value_enum)]
    platform: Option<TargetPlatform>,

    /// Backend to use (overrides default for platform).
    /// Example: `--platform linux --backend hydrolysis`.
    #[arg(short, long, value_enum)]
    backend: Option<TargetBackend>,

    /// Android painter the Hydrolysis host draws with (gpu, hwui).
    /// Only valid with `--platform android --backend hydrolysis`; the
    /// `[hydrolysis] painter` table in `Water.toml` is the project
    /// default when omitted.
    #[arg(long, value_enum)]
    painter: Option<HydrolysisAndroidPainter>,

    /// Device identifier (if not specified, uses first available device).
    #[arg(short, long)]
    device: Option<String>,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,

    /// Minimum log level to display (error, warn, info, debug, verbose).
    /// Streams device logs at or above this level and has the application log
    /// at it, so `debug` shows its `tracing::debug!` output.
    #[arg(long, value_enum)]
    logs: Option<CliLogLevel>,

    /// Include all native platform logs (`NSLog`, `print`, etc.), not just `WaterUI` logs.
    /// This is noisy but useful for debugging native code issues.
    #[arg(long)]
    native_logs: bool,

    /// Set an environment variable inside the launched application, as
    /// `KEY=VALUE`. Repeatable. Delivered through each platform's launch
    /// channel: `SIMCTL_CHILD_*` on Apple platforms and `waterui.env.*` intent
    /// extras on Android, where `Os.setenv` applies them before the app starts.
    #[arg(long = "env", value_name = "KEY=VALUE", value_parser = parse_env_assignment)]
    env: Vec<(String, String)>,

    /// Run the app in this terminal with the experimental TUI backend.
    ///
    /// The terminal becomes the application surface: `water` hands the TTY to
    /// the built launcher binary, so device and log streaming flags do not
    /// apply. The backend is pinned to a fixed `water-rs/tui` revision; set
    /// `WATERUI_TUI_PATH` to a local checkout when developing the backend.
    #[arg(
        long,
        conflicts_with_all = ["platform", "backend", "device", "logs", "native_logs"]
    )]
    tui: bool,

    /// Build in release mode (optimized). The web dev server never runs in a
    /// release build: `include_web!` renders the staged bundle instead.
    #[arg(long)]
    release: bool,

    /// Build for profiling: full release optimization with debug info and
    /// symbols kept so a profiler can symbolicate the recording.
    #[arg(long, conflicts_with = "release")]
    profiling: bool,

    /// Build fully unoptimized, skipping the light optimization `water run`
    /// applies by default to backends whose per-frame cost is high.
    #[arg(long, conflicts_with_all = ["release", "profiling"])]
    debug: bool,

    /// Do not start the frontend dev server for `include_web!` mounts; the
    /// debug build renders the staged bundle, packaged as `water package`
    /// does.
    #[arg(long)]
    no_dev_server: bool,

    /// Skip the confirmation prompt required by experimental backends
    /// (needed in non-interactive environments).
    #[arg(short = 'y', long)]
    yes: bool,
}

/// Parses one `--env KEY=VALUE` argument into its key and value.
///
/// The key may not be empty or contain `=`; everything after the first `=` is
/// the value, so values may hold further `=` characters.
fn parse_env_assignment(raw: &str) -> Result<(String, String), String> {
    let (key, value) = raw
        .split_once('=')
        .ok_or_else(|| String::from("expected KEY=VALUE"))?;
    if key.is_empty() {
        return Err(String::from("expected KEY=VALUE with a non-empty key"));
    }
    Ok((key.to_string(), value.to_string()))
}

/// Log level for filtering device logs (CLI argument wrapper).
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum CliLogLevel {
    /// Only errors
    Error,
    /// Warnings and errors
    Warn,
    /// Info, warnings, and errors
    #[default]
    Info,
    /// Debug and above
    Debug,
    /// All logs including verbose
    Verbose,
}

impl From<CliLogLevel> for LogLevel {
    fn from(level: CliLogLevel) -> Self {
        match level {
            CliLogLevel::Error => Self::Error,
            CliLogLevel::Warn => Self::Warn,
            CliLogLevel::Info => Self::Info,
            CliLogLevel::Debug => Self::Debug,
            CliLogLevel::Verbose => Self::Verbose,
        }
    }
}

/// Resolve the effective backend for a platform.
/// Returns the backend to use and validates compatibility.
fn resolve_backend(
    platform: TargetPlatform,
    backend_override: Option<TargetBackend>,
) -> Result<TargetBackend> {
    // Default backends for each platform
    let default_backend = match platform {
        TargetPlatform::Ios | TargetPlatform::Macos => TargetBackend::Apple,
        TargetPlatform::Android => TargetBackend::Android,
        TargetPlatform::Linux | TargetPlatform::Windows | TargetPlatform::Web => {
            TargetBackend::Hydrolysis
        }
        TargetPlatform::Esp32s3 | TargetPlatform::Esp32c3 | TargetPlatform::Esp32p4 => {
            TargetBackend::Dew
        }
    };

    let backend = backend_override.unwrap_or(default_backend);

    // Validate backend supports platform
    let supported = matches!(
        (platform, backend),
        (TargetPlatform::Ios, TargetBackend::Apple)
            | (
                TargetPlatform::Macos,
                TargetBackend::Apple | TargetBackend::Hydrolysis
            )
            | (
                TargetPlatform::Android,
                TargetBackend::Android | TargetBackend::Hydrolysis
            )
            | (
                TargetPlatform::Linux,
                TargetBackend::Gtk4 | TargetBackend::Hydrolysis
            )
            | (
                TargetPlatform::Windows,
                TargetBackend::Hydrolysis | TargetBackend::WinUi
            )
            | (TargetPlatform::Web, TargetBackend::Hydrolysis)
            | (
                TargetPlatform::Esp32s3 | TargetPlatform::Esp32c3 | TargetPlatform::Esp32p4,
                TargetBackend::Dew
            )
    );

    if !supported {
        bail!(
            "Backend {:?} does not support platform {:?}.\n\
             Valid combinations:\n  \
             - iOS: apple\n  \
             - macOS: apple, hydrolysis\n  \
             - Android: android, hydrolysis\n  \
             - Linux: gtk4, hydrolysis\n  \
             - Windows: hydrolysis, winui\n  \
             - Web: hydrolysis\n  \
             - ESP32-S3: dew\n  \
             - ESP32-C3: dew\n  \
             - ESP32-P4: dew",
            backend,
            platform
        );
    }

    Ok(backend)
}

/// The backend a run on `platform` uses when `--backend` is not given.
const fn default_backend(platform: TargetPlatform) -> TargetBackend {
    match platform {
        TargetPlatform::Ios | TargetPlatform::Macos => TargetBackend::Apple,
        TargetPlatform::Android => TargetBackend::Android,
        TargetPlatform::Linux | TargetPlatform::Windows | TargetPlatform::Web => {
            TargetBackend::Hydrolysis
        }
        TargetPlatform::Esp32s3 | TargetPlatform::Esp32c3 | TargetPlatform::Esp32p4 => {
            TargetBackend::Dew
        }
    }
}

/// The managed native backends a run on `platform` and `backend` needs
/// opened.
///
/// `--platform ios` covers both the physical device and the simulator; the
/// device that decides between them is selected only after the project is
/// open, and both build with the same Apple project. Hydrolysis on Android
/// opens nothing — the old widget-FFI backend is not its runtime; the
/// managed launcher crate `ensure_generated_backend` produces is.
const fn managed_backends(platform: TargetPlatform, backend: TargetBackend) -> ManagedBackends {
    if matches!(platform, TargetPlatform::Android) && matches!(backend, TargetBackend::Hydrolysis) {
        return ManagedBackends::NONE;
    }
    ManagedBackends::for_platform(lib_platform(platform))
}

const fn lib_platform(platform: TargetPlatform) -> LibTargetPlatform {
    match platform {
        TargetPlatform::Ios => LibTargetPlatform::IOS,
        TargetPlatform::Macos => LibTargetPlatform::MacOS,
        TargetPlatform::Android => LibTargetPlatform::Android,
        TargetPlatform::Linux => LibTargetPlatform::Linux,
        TargetPlatform::Windows => LibTargetPlatform::Windows,
        TargetPlatform::Web => LibTargetPlatform::Web,
        TargetPlatform::Esp32s3 => LibTargetPlatform::Esp32S3,
        TargetPlatform::Esp32c3 => LibTargetPlatform::Esp32C3,
        TargetPlatform::Esp32p4 => LibTargetPlatform::Esp32P4,
    }
}

const fn resolve_platform(platform_override: Option<TargetPlatform>) -> TargetPlatform {
    if let Some(platform) = platform_override {
        return platform;
    }

    #[cfg(target_os = "macos")]
    {
        TargetPlatform::Macos
    }
    #[cfg(target_os = "linux")]
    {
        TargetPlatform::Linux
    }
    #[cfg(target_os = "windows")]
    {
        TargetPlatform::Windows
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        panic!(
            "`water run` could not determine a default platform for this host. Please pass --platform explicitly."
        );
    }
}

/// Run the run command.
///
/// `interrupts` is the process's Ctrl-C channel: the supervised run owns
/// its shutdown through [`Running::supervise`], and the non-supervised
/// paths (web dev server, esp32) race it so an interrupt ends them the
/// way the command-level cancel used to.
pub async fn run(shell: &Shell, args: Args, interrupts: smol::channel::Receiver<()>) -> Result<()> {
    if args.tui {
        return Box::pin(crate::until_interrupt(
            Box::pin(run_tui_app(shell, args)),
            &interrupts,
        ))
        .await
        .map(|_| ());
    }

    let host = waterui_cli::toolchain::Host::current();
    let launch = Box::pin(async {
        let Some(context) = Box::pin(prepare_run_context(shell, &args)).await? else {
            return Ok(None);
        };
        print_run_header(shell, &context);
        check_run_toolchain(shell, &host, context.platform, context.backend).await?;

        if context.platform == TargetPlatform::Web {
            run_web_app(shell, &context.project).await?;
            return Ok(None);
        }
        if context.platform.esp32_chip().is_some() {
            Box::pin(run_esp32_app(
                shell,
                &context.project,
                args.device.as_deref(),
            ))
            .await?;
            return Ok(None);
        }

        let selection = Box::pin(select_run_device(
            shell,
            &host,
            context.platform,
            context.backend,
            &context.project,
            args.device.as_deref(),
        ))
        .await?;
        let config = build_run_config(shell, &host, &args, &context.project, context.backend).await;

        #[cfg(target_os = "macos")]
        let crash_ctx = match CrashReportContext::try_new(
            &context.project,
            context.platform,
            context.backend,
        ) {
            Ok(ctx) => ctx,
            Err(e) => {
                warn!(shell, "Crash report augmentation disabled: {e}");
                None
            }
        };

        // The dev-server guard is held for the app's whole run. Cancelling
        // this future drops it and the app's monitor acknowledges the kill.
        let (running, dev_server) = Box::pin(shell.display_output(build_and_run(
            shell,
            &host,
            &context.project,
            context.platform,
            context.backend,
            selection,
            config,
        )))
        .await?;
        Ok(Some(RunReady {
            backend: context.backend,
            running,
            dev_server,
            #[cfg(target_os = "macos")]
            crash_ctx,
        }))
    });

    let Some(Some(RunReady {
        backend,
        running,
        dev_server: _dev_server,
        #[cfg(target_os = "macos")]
        mut crash_ctx,
    })) = crate::until_interrupt(launch, &interrupts).await?
    else {
        return Ok(());
    };

    line!(shell);
    note!(shell, "Press Ctrl+C to stop the application");
    line!(shell);

    // Stream device events
    #[cfg(target_os = "macos")]
    stream_running_events(shell, &host, running, interrupts, backend, &mut crash_ctx).await?;
    #[cfg(not(target_os = "macos"))]
    stream_running_events(shell, running, interrupts, backend).await?;

    Ok(())
}

struct RunReady {
    backend: TargetBackend,
    running: Running,
    dev_server: Option<web::WebDevServer>,
    #[cfg(target_os = "macos")]
    crash_ctx: Option<CrashReportContext>,
}

/// Build and launch the experimental TUI backend in the invoking terminal.
///
/// This path bypasses the device pipeline entirely: the generated launcher is
/// a plain host binary that owns the TTY, so `water` hands the terminal over
/// after the build instead of streaming events through a device abstraction.
async fn run_tui_app(shell: &Shell, args: Args) -> Result<()> {
    use std::io::IsTerminal as _;

    warn!(
        shell,
        "The TUI backend is experimental — unsupported views panic instead of degrading"
    );
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("the TUI backend requires an interactive terminal");
    }
    if !super::confirm_experimental_backend(shell, "TUI", args.yes)? {
        return Ok(());
    }

    let project_path = crate::project_path::canonicalize(&args.path)?;
    let project = Box::pin(Project::open(&project_path, ManagedBackends::NONE)).await?;
    let launcher_dir = Box::pin(waterui_cli::tui::ensure_launcher(&project)).await?;

    let sccache_path = detect_sccache_path(shell, &waterui_cli::toolchain::Host::current()).await;
    let binary = shell
        .display_output(Box::pin(waterui_cli::tui::build(
            &project,
            &launcher_dir,
            sccache_path,
            Some(shell.build_progress()),
        )))
        .await?;

    note!(
        shell,
        "The TUI backend replaces this terminal until the app exits"
    );
    shell.clear();
    waterui_cli::tui::exec(&binary)
}

async fn prepare_run_context(shell: &Shell, args: &Args) -> Result<Option<RunContext>> {
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let platform = resolve_platform(args.platform);
    waterui_cli::platform::ensure_desktop_platform_is_host(
        platform.desktop_os(),
        std::env::consts::OS,
    )?;
    let backend = resolve_backend(
        platform,
        Some(args.backend.unwrap_or_else(|| default_backend(platform))),
    )?;
    let managed_backends = managed_backends(platform, backend);
    let mut project = Box::pin(Project::open(&project_path, managed_backends)).await?;
    if project.manifest().package.embedded {
        bail!(
            "`water run` does not apply to embedded projects: the crate is a library the host app embeds — build the artifact with `water build` and run the host app"
        );
    }

    backend
        .lib_backend()
        .validate_host_support(lib_platform(platform))?;
    validate_device_arg(platform, backend, args.device.as_deref())?;
    validate_log_pipeline_args(platform, args.logs, args.native_logs)?;
    if args.painter.is_some()
        && !(platform == TargetPlatform::Android && backend == TargetBackend::Hydrolysis)
    {
        bail!("--painter only applies to `--platform android --backend hydrolysis`");
    }

    if backend.is_experimental()
        && !super::confirm_experimental_backend(shell, backend_name(backend), args.yes)?
    {
        return Ok(None);
    }

    // Selecting an ESP32 platform pins the chip so the generated harness and
    // build target follow the platform (the chip drives the target triple,
    // QEMU model, and firmware parameters).
    if let Some(chip) = platform.esp32_chip() {
        project.set_esp32_chip(chip).await?;
    }

    let project = Box::pin(super::ensure_generated_backend(shell, project, backend)).await?;

    Ok(Some(RunContext {
        project,
        platform,
        backend,
    }))
}

fn print_run_header(shell: &Shell, context: &RunContext) {
    header!(
        shell,
        "Running {} on {} ({})",
        context.project.crate_name(),
        platform_name(context.platform),
        backend_name(context.backend)
    );
}

async fn check_run_toolchain(
    shell: &Shell,
    host: &waterui_cli::toolchain::Host,
    platform: TargetPlatform,
    backend: TargetBackend,
) -> Result<()> {
    let spinner = shell.spinner("Checking toolchain...");
    check_toolchain_for_backend(host, platform, backend).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "Toolchain ready");
    Ok(())
}

async fn run_web_app(shell: &Shell, project: &Project) -> Result<()> {
    let spinner = shell.spinner("Building Hydrolysis web app...");
    let site_root = prepare_hydrolysis_web_dev_site(project).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "Built Hydrolysis web app at {}", site_root.display());

    let server = HydrolysisWebDevServer::start(site_root).await?;
    line!(shell);
    note!(shell, "Serving at http://{}/", server.address());
    note!(shell, "Press Ctrl+C to stop the web server");
    let _server = server;
    futures_util::future::pending::<()>().await;
    unreachable!("web dev server future should be cancelled by Ctrl+C")
}

async fn run_esp32_app(shell: &Shell, project: &Project, device: Option<&str>) -> Result<()> {
    let sccache_path = detect_sccache_path(shell, &waterui_cli::toolchain::Host::current()).await;
    let build_options = sccache_path
        .map_or_else(
            || BuildOptions::development(BuildProfile::Debug),
            |sccache| BuildOptions::development(BuildProfile::Debug).with_sccache(sccache),
        )
        .with_progress(shell.build_progress());

    let _ = shell.status(">", "Building ESP32 firmware...");
    shell
        .display_output(Box::pin(run_esp32(project, build_options, device)))
        .await
}

async fn select_run_device(
    shell: &Shell,
    host: &waterui_cli::toolchain::Host,
    platform: TargetPlatform,
    backend: TargetBackend,
    project: &Project,
    device_id: Option<&str>,
) -> Result<DeviceSelection> {
    let device = find_device(shell, host, platform, backend, project, device_id).await?;

    let needs_launch = device.needs_launch();
    if needs_launch {
        note!(shell, "Will launch: {}", device_name(&device));
    } else {
        success!(shell, "Found device: {}", device_name(&device));
    }

    Ok(DeviceSelection {
        device,
        needs_launch,
    })
}

async fn build_run_config(
    shell: &Shell,
    host: &waterui_cli::toolchain::Host,
    args: &Args,
    project: &Project,
    backend: TargetBackend,
) -> BuildRunConfig {
    let sccache_path = detect_sccache_path(shell, host).await;
    let mut run_options = RunOptions::new();
    if let Some(level) = args.logs.map(LogLevel::from) {
        run_options.set_log_level(level);
    }
    run_options.set_native_logs(args.native_logs);
    run_options.describe_project(project);
    // Explicit `--env` wins over the derived project defaults above.
    for (key, value) in &args.env {
        run_options.insert_env_var(key.clone(), value.clone());
    }

    let profile = run_profile(args, backend);
    BuildRunConfig {
        run_options,
        sccache_path,
        profile,
        dev_server: !profile.is_release() && !args.no_dev_server,
        painter: hydrolysis_android::resolve_painter(project, args.painter),
    }
}

/// Resolve the Cargo profile `water run` builds under.
///
/// With no profile flag the backend's default development profile applies —
/// the same default `water build` uses, so a run reuses the units an earlier
/// build compiled into the shared target directory. `--debug` opts back into
/// a fully unoptimized build; `--release` and `--profiling` select the
/// release profile without and with debug info.
const fn run_profile(args: &Args, backend: TargetBackend) -> BuildProfile {
    if args.release {
        BuildProfile::Release
    } else if args.profiling {
        BuildProfile::Profiling
    } else if args.debug {
        BuildProfile::Debug
    } else {
        backend.lib_backend().default_development_profile()
    }
}

/// Print the run's device events until the run ends.
///
/// Shutdown policy lives in [`Running::supervise`]: the first interrupt
/// stops the app and keeps the stream alive for shutdown output, a second
/// kills it, and queued events behind a terminal event still arrive
/// before the stream ends. This loop prints, and fails the run once it ends
/// if the monitor reported an error.
#[cfg(target_os = "macos")]
async fn stream_running_events(
    shell: &Shell,
    host: &waterui_cli::toolchain::Host,
    running: Running,
    interrupts: smol::channel::Receiver<()>,
    backend: TargetBackend,
    crash_ctx: &mut Option<CrashReportContext>,
) -> Result<()> {
    let backend_log_name = backend_name(backend);
    let mut events = std::pin::pin!(running.supervise(interrupts));
    let mut monitor_errors = MonitorErrors::default();

    while let Some(event) = events.next().await {
        monitor_errors.observe(&event);
        if matches!(event, DeviceEvent::Started)
            && let Some(ctx) = crash_ctx.as_mut()
        {
            ctx.refresh_start();
        }

        let event = if let Some(ctx) = crash_ctx.as_ref() {
            augment_event_with_crash_report(host, event, ctx).await
        } else {
            event
        };

        handle_device_event(shell, event, backend_log_name)?;
    }
    monitor_errors.into_result()
}

/// Print the run's device events until the run ends — the non-macOS
/// twin of the function above, without crash-report augmentation.
#[cfg(not(target_os = "macos"))]
async fn stream_running_events(
    shell: &Shell,
    running: Running,
    interrupts: smol::channel::Receiver<()>,
    backend: TargetBackend,
) -> Result<()> {
    let backend_log_name = backend_name(backend);
    let mut events = std::pin::pin!(running.supervise(interrupts));
    let mut monitor_errors = MonitorErrors::default();

    while let Some(event) = events.next().await {
        monitor_errors.observe(&event);
        handle_device_event(shell, event, backend_log_name)?;
    }
    monitor_errors.into_result()
}

/// The number of monitor errors; each message is printed as it arrives.
#[derive(Default)]
struct MonitorErrors(usize);

impl MonitorErrors {
    const fn observe(&mut self, event: &DeviceEvent) {
        if matches!(event, DeviceEvent::MonitorError { .. }) {
            self.0 += 1;
        }
    }

    fn into_result(self) -> Result<()> {
        if self.0 == 0 {
            Ok(())
        } else {
            bail!("the run's monitor reported {} error(s)", self.0)
        }
    }
}

#[cfg(target_os = "macos")]
async fn augment_event_with_crash_report(
    host: &waterui_cli::toolchain::Host,
    event: DeviceEvent,
    ctx: &CrashReportContext,
) -> DeviceEvent {
    match event {
        DeviceEvent::Exited(exit) => find_latest_ips_report(host, ctx)
            .await
            .map_or(DeviceEvent::Exited(exit), |report| {
                DeviceEvent::Crashed(Crash::new(CrashCause::Report(Box::new(report))))
            }),
        DeviceEvent::Crashed(crash)
            if crash.report.is_none() && !matches!(crash.cause, CrashCause::Report(_)) =>
        {
            let Some(report) = find_latest_ips_report(host, ctx).await else {
                return DeviceEvent::Crashed(crash);
            };
            DeviceEvent::Crashed(Crash {
                report: Some(Box::new(report)),
                ..crash
            })
        }
        other => other,
    }
}

/// Build, package, and run on device.
async fn build_and_run(
    shell: &Shell,
    host: &waterui_cli::toolchain::Host,
    project: &Project,
    cli_platform: TargetPlatform,
    backend: TargetBackend,
    selection: DeviceSelection,
    config: BuildRunConfig,
) -> Result<(Running, Option<web::WebDevServer>)> {
    let build_plan = resolve_build_plan(cli_platform, &selection.device);
    let physical_ios = is_physical_ios(&selection.device);
    // The provisioning profile for a physical-device run must name the
    // destination's hardware UDID — capture it before the selection is
    // consumed by the launch task.
    let device_udid = match &selection.device {
        SelectedDevice::ApplePhysical(device) => Some(device.udid.clone()),
        _ => None,
    };
    let launch_task =
        spawn_device_launch_task(host.clone(), selection.device, selection.needs_launch);

    // The package options and the release-signing plan are resolved before
    // the Rust build: a release run without a valid `[signing.android]`
    // fails here instead of after compiling. The plan binds to this project
    // and these options; the Gradle step re-proves the binding rather than
    // resolving again. `[signing.android]` is an Android platform contract —
    // both Android backends prepare it.
    let package_options =
        package_options(&config, shell.build_progress()).with_device_udid(device_udid);
    let prepared_signing = (build_plan.lib_platform == LibTargetPlatform::Android)
        .then(|| waterui_cli::android::signing::PreparedSigning::resolve(project, &package_options))
        .transpose()?;

    let _ = shell.status(">", "Building...");
    let built = Box::pin(build_for_backend(
        project,
        backend,
        &build_plan,
        build_options(&config).with_progress(shell.build_progress()),
    ))
    .await?;

    // A declared `include_web!` mount is served by the bundler's own dev
    // server in debug runs — spawn it after the Rust build (its root comes
    // from the compiled metadata) and before packaging, so the packaged app
    // stages no web output.
    let dev_server = if config.dev_server {
        start_web_dev_server(shell, project, &built, physical_ios).await?
    } else {
        None
    };

    let _ = shell.status(">", "Packaging...");
    let artifact = Box::pin(package_for_backend(
        project,
        backend,
        &build_plan,
        &built,
        package_options,
        prepared_signing.as_ref(),
        config.painter,
    ))
    .await?;

    if selection.needs_launch {
        let _ = shell.status(">", "Waiting for device...");
    }
    let device = launch_task.await?;

    let mut run_options = config.run_options;
    if let Some(server) = &dev_server {
        apply_dev_url_handoff(&device, server.url(), &mut run_options)?;
    }

    let _ = shell.status(">", format!("Running {}", artifact.path().display()));
    let running = run_with_options(host, device, artifact, run_options).await?;

    Ok((running, dev_server))
}

/// Spawn the declared frontend's dev server for a debug run, when the
/// compiled library mounts one with `include_web!`. `built` is the build the
/// run just produced — its app library's `waterui_meta_bundle_*` statics
/// declare the mount.
async fn start_web_dev_server(
    shell: &Shell,
    project: &Project,
    built: &waterui_cli::build::BuiltTarget,
    expose_on_lan: bool,
) -> Result<Option<web::WebDevServer>> {
    let Some(meta) = web::web_mount(&built.app_symbols()?)? else {
        return Ok(None);
    };
    let root = meta
        .project
        .as_ref()
        .expect("web_mount only returns a mount that declares a project");
    let package_manager = project
        .manifest()
        .web
        .as_ref()
        .map_or_else(web::PackageManager::default, |web| web.package_manager);
    if !package_manager.is_installed().await {
        bail!(
            "`{}` is not installed; run `water doctor`",
            package_manager.binary()
        );
    }
    let script = web::dev_script(root)?;
    let _ = shell.status(
        ">",
        format!("Starting `{} run {script}`", package_manager.binary()),
    );
    let server = web::WebDevServer::spawn(package_manager, root, &script, expose_on_lan).await?;
    let _ = shell.status(">", format!("Dev server ready at {}", server.url()));
    Ok(Some(server))
}

/// Route the dev-server URL into the selected target's launch channel.
fn apply_dev_url_handoff(
    device: &SelectedDevice,
    url: &url::Url,
    run_options: &mut RunOptions,
) -> Result<()> {
    let target = match device {
        SelectedDevice::Local(_) => web::DevTarget::Desktop,
        SelectedDevice::AppleSimulator(_) => web::DevTarget::IosSimulator,
        SelectedDevice::ApplePhysical(_) => web::DevTarget::IosDevice,
        SelectedDevice::AndroidDevice(_) | SelectedDevice::AndroidEmulator(_) => {
            web::DevTarget::Android
        }
    };
    // Every launch path forwards `WATERUI_DEV_URL` in the environment —
    // `cmd.env`/`open --env` on desktop, `SIMCTL_CHILD_*` under `simctl
    // launch`, `devicectl -e` on a physical device, and the `waterui.env.`
    // intent extra on Android (where `run_on_android` additionally makes the
    // port reachable with `adb reverse`). A physical device cannot reach the
    // Mac's loopback, so its URL is rewritten to the LAN address first.
    let url = web::device_facing_url(target, url)?;
    run_options.insert_env_var(web::DEV_URL_ENV.to_string(), url.to_string());
    Ok(())
}

fn resolve_build_plan(cli_platform: TargetPlatform, device: &SelectedDevice) -> BuildPlan {
    let lib_platform = match cli_platform {
        // The device decides the iOS SDK: a physical device builds
        // `aarch64-apple-ios` (iphoneos), a simulator `*-apple-ios-sim`.
        TargetPlatform::Ios => {
            if is_physical_ios(device) {
                LibTargetPlatform::IOS
            } else {
                LibTargetPlatform::IOSSimulator
            }
        }
        TargetPlatform::Macos => LibTargetPlatform::MacOS,
        TargetPlatform::Android => LibTargetPlatform::Android,
        TargetPlatform::Linux => LibTargetPlatform::Linux,
        TargetPlatform::Windows => LibTargetPlatform::Windows,
        TargetPlatform::Web => panic!("web run should not enter build_and_run"),
        TargetPlatform::Esp32s3 | TargetPlatform::Esp32c3 | TargetPlatform::Esp32p4 => {
            panic!("esp32 run should not enter build_and_run")
        }
    };
    let android_abi = device_android_abi(device);

    BuildPlan {
        lib_platform,
        android_abi,
    }
}

/// The Android ABI the selected device builds and packages for, when it is
/// one.
///
/// The ABI is a property of the device, not the backend: Hydrolysis on a
/// desktop target selects the local machine, which has no ABI.
const fn device_android_abi(
    device: &SelectedDevice,
) -> Option<waterui_cli::android::platform::AndroidAbi> {
    match device {
        SelectedDevice::AndroidDevice(dev) => Some(dev.abi()),
        SelectedDevice::AndroidEmulator(emu) => Some(emu.expected_abi()),
        _ => None,
    }
}

fn spawn_device_launch_task(
    host: waterui_cli::toolchain::Host,
    device: SelectedDevice,
    needs_launch: bool,
) -> smol::Task<Result<SelectedDevice>> {
    smol::spawn(async move {
        if needs_launch {
            match &device {
                SelectedDevice::AppleSimulator(sim) => sim.launch(&host).await?,
                SelectedDevice::ApplePhysical(dev) => dev.launch(&host).await?,
                SelectedDevice::Local(local) => local.launch(&host).await?,
                SelectedDevice::AndroidDevice(dev) => dev.launch(&host).await?,
                SelectedDevice::AndroidEmulator(emu) => emu.launch(&host).await?,
            }
        }
        Ok(device)
    })
}

fn build_options(config: &BuildRunConfig) -> BuildOptions {
    config
        .sccache_path
        .as_ref()
        .map_or_else(
            || BuildOptions::development(config.profile),
            |sccache| BuildOptions::development(config.profile).with_sccache(sccache.clone()),
        )
        .with_dev_server(config.dev_server)
}

async fn build_for_backend(
    project: &Project,
    backend: TargetBackend,
    plan: &BuildPlan,
    build_options: BuildOptions,
) -> Result<waterui_cli::build::BuiltTarget> {
    match backend {
        TargetBackend::Apple => {
            Box::pin(build_rust_lib(project, plan.lib_platform, build_options)).await
        }
        TargetBackend::Android => {
            let abi = plan
                .android_abi
                .ok_or_else(|| eyre::eyre!("Internal error: missing Android ABI for build"))?;
            AndroidPlatform::clean_jni_libs(project).await?;
            Box::pin(AndroidPlatform::new(abi).build(project, build_options)).await
        }
        TargetBackend::Gtk4 => Box::pin(build_gtk4(project, build_options)).await,
        TargetBackend::Hydrolysis => {
            if plan.lib_platform == LibTargetPlatform::Android {
                let abi = plan
                    .android_abi
                    .ok_or_else(|| eyre::eyre!("Internal error: missing Android ABI for build"))?;
                hydrolysis_android::clean_jni_libs(project).await?;
                Box::pin(hydrolysis_android::build(
                    project,
                    &waterui_cli::toolchain::Host::current(),
                    abi,
                    build_options,
                ))
                .await
            } else {
                Box::pin(build_hydrolysis(project, plan.lib_platform, build_options)).await
            }
        }
        TargetBackend::WinUi => Box::pin(build_winui(project, build_options)).await,
        TargetBackend::Dew => {
            panic!("esp32 run should not enter build_and_run")
        }
    }
}

fn package_options(config: &BuildRunConfig, progress: BuildProgress) -> PackageOptions {
    PackageOptions::development()
        .with_debug(!config.profile.is_release())
        .with_dev_server(config.dev_server)
        .with_progress(progress)
}

async fn package_for_backend(
    project: &Project,
    backend: TargetBackend,
    plan: &BuildPlan,
    built: &waterui_cli::build::BuiltTarget,
    package_options: PackageOptions,
    prepared_signing: Option<&waterui_cli::android::signing::PreparedSigning>,
    painter: HydrolysisAndroidPainter,
) -> Result<Artifact> {
    match backend {
        TargetBackend::Apple => {
            Box::pin(package_apple(
                project,
                plan.lib_platform,
                package_options,
                built,
            ))
            .await
        }
        TargetBackend::Android => {
            let abi = plan
                .android_abi
                .ok_or_else(|| eyre::eyre!("Internal error: missing Android ABI for packaging"))?;
            Box::pin(AndroidPlatform::package_with_abis(
                project,
                package_options,
                &[abi],
                built,
                prepared_signing.ok_or_else(|| {
                    eyre::eyre!("Internal error: Android packaging has no signing plan")
                })?,
            ))
            .await
        }
        TargetBackend::Gtk4 => Box::pin(package_gtk4(project, package_options, built)).await,
        TargetBackend::Hydrolysis => {
            if plan.lib_platform == LibTargetPlatform::Android {
                let abi = plan.android_abi.ok_or_else(|| {
                    eyre::eyre!("Internal error: missing Android ABI for packaging")
                })?;
                Box::pin(hydrolysis_android::package_with_abis(
                    project,
                    &waterui_cli::toolchain::Host::current(),
                    painter,
                    &package_options,
                    &[abi],
                    built,
                    prepared_signing.ok_or_else(|| {
                        eyre::eyre!("Internal error: Android packaging has no signing plan")
                    })?,
                ))
                .await
            } else {
                Box::pin(package_hydrolysis(
                    project,
                    plan.lib_platform,
                    package_options,
                    Some(built),
                ))
                .await
            }
        }
        TargetBackend::WinUi => Box::pin(package_winui(project, package_options, built)).await,
        TargetBackend::Dew => panic!("esp32 run should not enter build_and_run"),
    }
}

struct BuildRunConfig {
    run_options: RunOptions,
    sccache_path: Option<PathBuf>,
    /// The Cargo profile the run builds under — see `run_profile`.
    profile: BuildProfile,
    /// Whether a declared `include_web!` mount may be served by the bundler's
    /// dev server: non-release profiles only, unless `--no-dev-server` opts
    /// out.
    dev_server: bool,
    /// The Hydrolysis Android painter the run packages with — the `--painter`
    /// override, `[hydrolysis] painter`, or the GPU default.
    painter: HydrolysisAndroidPainter,
}

/// Run artifact on device.
async fn run_with_options(
    host: &waterui_cli::toolchain::Host,
    device: SelectedDevice,
    artifact: Artifact,
    run_options: RunOptions,
) -> Result<Running> {
    let running = match device {
        SelectedDevice::AppleSimulator(sim) => sim.run(host, artifact, run_options).await?,
        SelectedDevice::ApplePhysical(dev) => dev.run(host, artifact, run_options).await?,
        SelectedDevice::Local(local) => local.run(host, artifact, run_options).await?,
        SelectedDevice::AndroidDevice(dev) => dev.run(host, artifact, run_options).await?,
        SelectedDevice::AndroidEmulator(emu) => emu.run(host, artifact, run_options).await?,
    };

    Ok(running)
}

/// A device that can be selected for running.
enum SelectedDevice {
    AppleSimulator(AppleSimulator),
    /// A paired physical iOS device (USB or "Connect via network").
    ApplePhysical(ApplePhysicalDevice),
    /// Local machine - used for desktop backends and macOS Apple backend.
    Local(Local),
    AndroidDevice(AndroidDevice),
    AndroidEmulator(AndroidEmulator),
}

impl SelectedDevice {
    /// Check if the device needs to be launched before running.
    fn needs_launch(&self) -> bool {
        match self {
            Self::AppleSimulator(sim) => sim.state != "Booted",
            Self::ApplePhysical(_) | Self::Local(_) | Self::AndroidDevice(_) => false,
            Self::AndroidEmulator(_) => true,
        }
    }
}

/// Whether the selected device is a physical iOS device — the one target
/// whose dev-server URL and launch channel differ from every other.
const fn is_physical_ios(device: &SelectedDevice) -> bool {
    matches!(device, SelectedDevice::ApplePhysical(_))
}

/// One selectable run target, flattened across simulators, physical devices,
/// and emulators so the remembered-device check and the picker see one list.
struct DeviceCandidate {
    device: SelectedDevice,
    /// Identifier persisted as the last-used device in `~/.water/config.toml`.
    id: String,
    /// Label shown in the interactive picker and non-interactive listings.
    label: String,
}

/// `~/.water/config.toml` key for the device a `(backend, platform)` last
/// ran on.
const fn device_memory_key(backend: TargetBackend, platform: TargetPlatform) -> &'static str {
    match (backend, platform) {
        (TargetBackend::Apple, TargetPlatform::Ios) => "apple/ios",
        (TargetBackend::Android, TargetPlatform::Android) => "android/android",
        (TargetBackend::Hydrolysis, TargetPlatform::Android) => "hydrolysis/android",
        // Device memory only exists for targets with a device dimension.
        _ => unreachable!(),
    }
}

/// The device last used for `key`, if the config records one. A config read
/// failure is a warning, not a run failure.
async fn remembered_device(key: &str) -> Option<String> {
    match waterui_cli::water_dir::ensure_global_config().await {
        Ok(config) => config.last_used_device.get(key).cloned(),
        Err(error) => {
            tracing::warn!("could not read the Water config for device memory: {error:#}");
            None
        }
    }
}

/// Record `id` as the last-used device for `key`. Best-effort: a config
/// write failure must not break a run.
async fn persist_device_choice(key: &str, id: &str) {
    match waterui_cli::water_dir::ensure_global_config().await {
        Ok(mut config) => {
            if config.last_used_device.get(key).map(String::as_str) == Some(id) {
                return;
            }
            config
                .last_used_device
                .insert(key.to_owned(), id.to_owned());
            if let Err(error) = waterui_cli::water_dir::write_global_config(&config).await {
                tracing::warn!("could not persist the last-used device: {error:#}");
            }
        }
        Err(error) => {
            tracing::warn!("could not read the Water config for device memory: {error:#}");
        }
    }
}

/// What `device_choice` resolved for a no-`--device` run.
#[derive(Debug)]
enum DeviceChoice {
    /// Run `candidates[index]`; `persist` records it as the last-used device.
    Use(usize, bool),
    /// The remembered device is gone — the caller warns and chooses among
    /// the candidates as if nothing were remembered.
    Stale(String),
    /// Multiple candidates and no memory — a human must pick.
    Prompt,
}

/// Pure selection rule: a remembered device still present wins; a single
/// candidate is unambiguous; otherwise the user must be asked.
fn device_choice(candidates: &[DeviceCandidate], remembered: Option<&str>) -> DeviceChoice {
    if let Some(id) = remembered {
        return candidates.iter().position(|c| c.id == id).map_or_else(
            || DeviceChoice::Stale(id.to_owned()),
            |index| DeviceChoice::Use(index, false),
        );
    }
    if candidates.len() == 1 {
        DeviceChoice::Use(0, false)
    } else {
        DeviceChoice::Prompt
    }
}

/// Ask which device to use. Non-interactive runs cannot pick, so they fail
/// with the candidate list.
fn prompt_for_device(
    shell: &Shell,
    prompt: &str,
    candidates: &[DeviceCandidate],
    spinner: Option<&indicatif::ProgressBar>,
) -> Result<usize> {
    use std::fmt::Write as _;
    if !shell.is_terminal() {
        let list = candidates.iter().fold(String::new(), |mut out, candidate| {
            write!(out, "\n  {} — {}", candidate.label, candidate.id)
                .expect("writing to a String cannot fail");
            out
        });
        bail!("Several devices are available; pass --device to choose one:{list}");
    }
    let labels: Vec<&str> = candidates.iter().map(|c| c.label.as_str()).collect();
    let pick = || {
        dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt(prompt)
            .items(&labels)
            .default(0)
            .interact()
    };
    // The scan spinner would redraw over the prompt; hide it while the user
    // answers.
    Ok(spinner.map_or_else(pick, |pb| pb.suspend(pick))?)
}

/// Pick one device out of `candidates`: the remembered last-used device when
/// it is still present, the single candidate when unambiguous, otherwise an
/// interactive picker. Choices the user or an explicit query make are
/// persisted; an unambiguous single candidate only writes when it repairs a
/// stale memory.
async fn choose_device_candidate(
    shell: &Shell,
    memory_key: &str,
    prompt: &str,
    candidates: Vec<DeviceCandidate>,
    spinner: Option<&indicatif::ProgressBar>,
) -> Result<SelectedDevice> {
    debug_assert!(!candidates.is_empty());

    let mut remembered = remembered_device(memory_key).await;
    let mut stale_memory = false;
    let (index, persist) = loop {
        match device_choice(&candidates, remembered.as_deref()) {
            DeviceChoice::Use(index, persist) => break (index, persist || stale_memory),
            DeviceChoice::Stale(id) => {
                warn!(shell, "Last-used device \"{id}\" is not available");
                remembered = None;
                stale_memory = true;
            }
            DeviceChoice::Prompt => {
                break (
                    prompt_for_device(shell, prompt, &candidates, spinner)?,
                    true,
                );
            }
        }
    };

    let candidate = candidates
        .into_iter()
        .nth(index)
        .expect("the index came from the candidate list");
    if persist {
        persist_device_choice(memory_key, &candidate.id).await;
    }
    Ok(candidate.device)
}

/// Select the iOS target. An explicit `--device` may name a paired physical
/// device or a simulator (matched by identifier, UDID, or name); without
/// one, the remembered last-used device wins, then an unambiguous single
/// candidate, then the picker.
async fn select_ios_device(
    shell: &Shell,
    host: &waterui_cli::toolchain::Host,
    project: &Project,
    device_id: Option<&str>,
    spinner: Option<&indicatif::ProgressBar>,
) -> Result<SelectedDevice> {
    let physical = ApplePhysicalDevice::scan(host)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!("devicectl device scan failed: {error:#}");
            Vec::new()
        });

    if let Some(query) = device_id {
        if let Some(device) = select_physical_ios(host, &physical, project, query).await? {
            persist_device_choice(
                device_memory_key(TargetBackend::Apple, TargetPlatform::Ios),
                &device.identifier,
            )
            .await;
            return Ok(SelectedDevice::ApplePhysical(device));
        }
        let sim = AppleSimulator::select_ios(host, project, Some(query)).await?;
        persist_device_choice(
            device_memory_key(TargetBackend::Apple, TargetPlatform::Ios),
            &sim.udid,
        )
        .await;
        return Ok(SelectedDevice::AppleSimulator(sim));
    }

    let candidates = ios_device_candidates(host, project, &physical).await?;
    if candidates.is_empty() {
        // Reuse the simulator path's diagnosis — it lists every simulator and
        // the required runtime.
        return Ok(SelectedDevice::AppleSimulator(
            AppleSimulator::select_ios(host, project, None).await?,
        ));
    }
    choose_device_candidate(
        shell,
        device_memory_key(TargetBackend::Apple, TargetPlatform::Ios),
        "Select an iOS device",
        candidates,
        spinner,
    )
    .await
}

/// Every device that can run the app on iOS: qualifying simulators (booted
/// first, the historic preference), then usable paired devices.
async fn ios_device_candidates(
    host: &waterui_cli::toolchain::Host,
    project: &Project,
    physical: &[ApplePhysicalDevice],
) -> Result<Vec<DeviceCandidate>> {
    let ios_version = |version: Option<&semver::Version>| {
        version.map_or_else(|| String::from("an unknown iOS"), |v| format!("iOS {v}"))
    };
    let deployment_target = |target_platform: LibTargetPlatform| async move {
        let (_, target) =
            waterui_cli::apple::platform::apple_deployment_target(project, target_platform).await?;
        waterui_cli::utils::parse_semver_version(&target).wrap_err_with(|| {
            format!("Failed to parse the project's IPHONEOS_DEPLOYMENT_TARGET `{target}`")
        })
    };
    let sim_target = deployment_target(LibTargetPlatform::IOSSimulator).await?;
    let device_target = deployment_target(LibTargetPlatform::IOS).await?;

    let mut simulators = AppleSimulator::scan_ios(host).await?;
    simulators.retain(|sim| sim.supports_deployment_target(&sim_target));
    simulators.sort_by_key(|sim| usize::from(sim.state != "Booted"));
    let mut candidates: Vec<DeviceCandidate> = simulators
        .into_iter()
        .map(|sim| DeviceCandidate {
            label: format!(
                "{} — {} ({})",
                sim.name,
                ios_version(sim.runtime_version.as_ref()),
                sim.state
            ),
            id: sim.udid.clone(),
            device: SelectedDevice::AppleSimulator(sim),
        })
        .collect();

    candidates.extend(
        physical
            .iter()
            .filter(|device| {
                device.usability().is_ok() && device.supports_deployment_target(&device_target)
            })
            .map(|device| DeviceCandidate {
                label: format!(
                    "{} — {} ({})",
                    device.name,
                    ios_version(device.os_version.as_ref()),
                    match device.transport {
                        waterui_cli::apple::physical::Transport::Wired => "USB",
                        waterui_cli::apple::physical::Transport::LocalNetwork => "Wi-Fi",
                        waterui_cli::apple::physical::Transport::Other => "paired",
                    }
                ),
                id: device.identifier.clone(),
                device: SelectedDevice::ApplePhysical(device.clone()),
            }),
    );
    Ok(candidates)
}

/// Match a `--device` query against paired physical devices.
///
/// Returns `Ok(None)` when nothing matches — the query may still name a
/// simulator. A matched device that cannot run the app (unreachable, no
/// Developer Mode, too-old OS) is an error naming the remedy rather than a
/// silent miss.
async fn select_physical_ios(
    host: &waterui_cli::toolchain::Host,
    devices: &[ApplePhysicalDevice],
    project: &Project,
    query: &str,
) -> Result<Option<ApplePhysicalDevice>> {
    let matches: Vec<&ApplePhysicalDevice> = devices
        .iter()
        .filter(|device| device.identifier == query || device.udid == query || device.name == query)
        .collect();
    let device = match matches.as_slice() {
        [] => return Ok(None),
        [device] => *device,
        candidates => {
            use std::fmt::Write as _;
            let list = candidates.iter().fold(String::new(), |mut out, device| {
                write!(out, "\n  {} ({})", device.name, device.identifier)
                    .expect("writing to a String cannot fail");
                out
            });
            bail!(
                "Device \"{query}\" matches {} devices; select one by identifier:{list}",
                candidates.len()
            );
        }
    };

    if let Err(reason) = device.usability() {
        bail!("{}", reason.remedy(device));
    }

    let (_, target) =
        waterui_cli::apple::platform::apple_deployment_target(project, LibTargetPlatform::IOS)
            .await?;
    let deployment_target =
        waterui_cli::utils::parse_semver_version(&target).wrap_err_with(|| {
            format!("Failed to parse the project's IPHONEOS_DEPLOYMENT_TARGET `{target}`")
        })?;
    if !device.supports_deployment_target(&deployment_target) {
        bail!(
            "{} runs {}, but this app requires iOS {deployment_target} (IPHONEOS_DEPLOYMENT_TARGET)",
            device.name,
            device.os_version.as_ref().map_or_else(
                || String::from("an unknown iOS version"),
                |v| format!("iOS {v}")
            ),
        );
    }

    // A device build must be signed; check the keychain has a development
    // identity now rather than after a multi-minute build.
    waterui_cli::apple::toolchain::development_team_id(host).await?;

    Ok(Some(device.clone()))
}

async fn check_toolchain_for_backend(
    host: &waterui_cli::toolchain::Host,
    platform: TargetPlatform,
    backend: TargetBackend,
) -> Result<()> {
    match backend {
        TargetBackend::Apple => {
            let sdk = match platform {
                TargetPlatform::Ios => AppleSdk::IosSimulator,
                TargetPlatform::Macos => AppleSdk::Macos,
                TargetPlatform::Android
                | TargetPlatform::Linux
                | TargetPlatform::Windows
                | TargetPlatform::Web
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
            toolchain_checks::check_android_run(host).await?;
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
                toolchain_checks::check_android_run(host).await?;
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
        TargetBackend::Dew => {
            if platform.esp32_chip().is_none() {
                bail!("Internal error: dew backend is not supported on {platform:?}");
            }
        }
    }
    Ok(())
}

async fn find_device(
    shell: &Shell,
    host: &waterui_cli::toolchain::Host,
    platform: TargetPlatform,
    backend: TargetBackend,
    project: &Project,
    device_id: Option<&str>,
) -> Result<SelectedDevice> {
    // For native desktop Rust backends, always use Local device regardless of platform.
    // Hydrolysis on Android is the exception: it goes through the same
    // device pipeline the Android backend uses.
    if backend == TargetBackend::Gtk4
        || (backend == TargetBackend::Hydrolysis && platform != TargetPlatform::Android)
        || backend == TargetBackend::WinUi
    {
        return Ok(SelectedDevice::Local(Local));
    }

    let spinner = shell.spinner("Scanning for devices...");
    let device = match platform {
        TargetPlatform::Ios => {
            select_ios_device(shell, host, project, device_id, spinner.as_ref()).await
        }
        TargetPlatform::Macos => {
            // macOS with Apple backend uses the local machine
            Ok(SelectedDevice::Local(Local))
        }
        TargetPlatform::Android => {
            select_android_device(shell, host, backend, device_id, spinner.as_ref()).await
        }
        TargetPlatform::Linux | TargetPlatform::Windows => {
            // Linux and Windows run on the local machine
            Ok(SelectedDevice::Local(Local))
        }
        TargetPlatform::Web => {
            bail!("web platform does not use the device pipeline");
        }
        TargetPlatform::Esp32s3 | TargetPlatform::Esp32c3 | TargetPlatform::Esp32p4 => {
            bail!("esp32 platform does not use the device pipeline");
        }
    };
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    device
}

/// Select the Android target. An explicit `--device` names a connected
/// device serial or an AVD; without one, the remembered last-used device
/// wins, then an unambiguous single candidate, then the picker.
async fn select_android_device(
    shell: &Shell,
    host: &waterui_cli::toolchain::Host,
    backend: TargetBackend,
    device_id: Option<&str>,
    spinner: Option<&indicatif::ProgressBar>,
) -> Result<SelectedDevice> {
    let key = device_memory_key(backend, TargetPlatform::Android);
    let devices = AndroidDevice::scan(host).await?;
    let avds = AndroidPlatform::list_avds(host).await?;

    if let Some(query) = device_id {
        for dev in devices {
            if dev.identifier() == query {
                persist_device_choice(key, dev.identifier()).await;
                return Ok(SelectedDevice::AndroidDevice(dev));
            }
        }
        if avds.iter().any(|avd| avd == query) {
            persist_device_choice(key, query).await;
            return Ok(SelectedDevice::AndroidEmulator(
                AndroidEmulator::open(host, query.to_string()).await?,
            ));
        }
        bail!("Device not found: {query}");
    }

    let mut candidates: Vec<DeviceCandidate> = devices
        .into_iter()
        .map(|dev| DeviceCandidate {
            label: format!("{} (connected)", dev.identifier()),
            id: dev.identifier().to_owned(),
            device: SelectedDevice::AndroidDevice(dev),
        })
        .collect();
    for avd in avds {
        candidates.push(DeviceCandidate {
            label: format!("{avd} (emulator)"),
            id: avd.clone(),
            device: SelectedDevice::AndroidEmulator(AndroidEmulator::open(host, avd).await?),
        });
    }

    if candidates.is_empty() {
        bail!(
            "No Android devices connected and no emulators available. Create an emulator with Android Studio or `avdmanager`, or connect a device."
        );
    }
    choose_device_candidate(shell, key, "Select an Android device", candidates, spinner).await
}

fn device_name(device: &SelectedDevice) -> String {
    match device {
        SelectedDevice::AppleSimulator(sim) => sim.name.clone(),
        SelectedDevice::ApplePhysical(dev) => dev.name.clone(),
        SelectedDevice::Local(local) => local.name().to_string(),
        SelectedDevice::AndroidDevice(dev) => dev.identifier().to_string(),
        SelectedDevice::AndroidEmulator(emu) => format!("{} (emulator)", emu.avd_name()),
    }
}

const fn platform_name(platform: TargetPlatform) -> &'static str {
    match platform {
        TargetPlatform::Ios => "iOS",
        TargetPlatform::Android => "Android",
        TargetPlatform::Macos => "macOS",
        TargetPlatform::Linux => "Linux",
        TargetPlatform::Windows => "Windows",
        TargetPlatform::Web => "Web",
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
        TargetBackend::Dew => "Dew",
    }
}

fn validate_device_arg(
    platform: TargetPlatform,
    backend: TargetBackend,
    device: Option<&str>,
) -> Result<()> {
    if device.is_none() {
        return Ok(());
    }

    if platform == TargetPlatform::Web {
        bail!("--device is not supported with the web platform");
    }

    // Targets without a device dimension always run on this machine; a
    // `--device` there can only be a mistake, so reject it instead of
    // silently ignoring it.
    let local_only = (matches!(backend, TargetBackend::Gtk4 | TargetBackend::Hydrolysis)
        && !(platform == TargetPlatform::Android && backend == TargetBackend::Hydrolysis))
        || (platform, backend) == (TargetPlatform::Macos, TargetBackend::Apple);
    if local_only {
        bail!("--device is not supported: this target always runs on the local machine");
    }
    Ok(())
}

fn validate_log_pipeline_args(
    platform: TargetPlatform,
    logs: Option<CliLogLevel>,
    native_logs: bool,
) -> Result<()> {
    let log_pipeline_unsupported = match platform {
        TargetPlatform::Web => Some("web"),
        // The serial monitor streams firmware logs directly.
        TargetPlatform::Esp32s3 => Some("esp32s3"),
        TargetPlatform::Esp32c3 => Some("esp32c3"),
        TargetPlatform::Esp32p4 => Some("esp32p4"),
        _ => None,
    };
    let Some(platform_label) = log_pipeline_unsupported else {
        return Ok(());
    };
    if logs.is_some() {
        bail!("--logs is not supported with the {platform_label} platform");
    }
    if native_logs {
        bail!("--native-logs is not supported with the {platform_label} platform");
    }
    Ok(())
}

/// Handle a device event.
///
fn handle_device_event(shell: &Shell, event: DeviceEvent, platform_name: &str) -> Result<()> {
    match event {
        DeviceEvent::Started => {
            let _ = shell.status("*", "Application started");
            Ok(())
        }
        DeviceEvent::Stopped => {
            let _ = shell.status("o", "Application stopped");
            Ok(())
        }
        DeviceEvent::Stdout { message } => {
            line!(shell, "[stdout] {message}");
            Ok(())
        }
        DeviceEvent::Stderr { message } => {
            warn!(shell, "[stderr] {message}");
            Ok(())
        }
        DeviceEvent::Log { level, message } => {
            let _ = shell.device_log(platform_name, level, message);
            Ok(())
        }
        DeviceEvent::MonitorError { message } => {
            error!(shell, "{message}");
            Ok(())
        }
        DeviceEvent::Exited(exit) => {
            let _ = shell.status("o", exit.terminal_message());
            Ok(())
        }
        DeviceEvent::Crashed(crash) => {
            if let CrashCause::Panic(panic) = &crash.cause {
                let extra = crash.panic_note();
                shell.panic(
                    panic,
                    extra.as_deref(),
                    crash
                        .report
                        .as_deref()
                        .map(waterui_cli::debug::CrashReport::log_path),
                );
            } else {
                error!(shell, "Application crashed: {crash}");
            }
            bail!("application crashed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Args, DeviceCandidate, DeviceChoice, SelectedDevice, TargetBackend, TargetPlatform,
        default_backend, device_android_abi, device_choice, handle_device_event, lib_platform,
        parse_env_assignment, prompt_for_device, resolve_backend, resolve_platform, run_profile,
        stream_running_events, validate_device_arg,
    };
    use clap::Parser as _;
    use waterui_cli::android::device::AndroidDevice;
    use waterui_cli::android::platform::AndroidAbi;
    use waterui_cli::build::BuildProfile;
    use waterui_cli::device::{ApplicationExit, DeviceEvent, Local, StopRequest};

    /// The run `Args` wrapped in a `Parser` so tests can exercise the real
    /// flag surface instead of constructing the clap struct field by field.
    #[derive(clap::Parser)]
    struct TestCli {
        #[command(flatten)]
        args: Args,
    }

    fn run_args(argv: &[&str]) -> Args {
        let mut full = vec!["water-run"];
        full.extend_from_slice(argv);
        TestCli::try_parse_from(full).expect("run args parse").args
    }

    #[test]
    fn run_profile_defaults_to_backend_development_profile() {
        // `water run` and `water build` share the generated crates' Cargo
        // target directory; if their flag-free defaults ever disagree, every
        // dependency unit re-fingerprints and the run cold-compiles the whole
        // graph (nightly "Fresh user / Windows" timeout).
        use clap::ValueEnum;
        let args = run_args(&[]);
        for backend in TargetBackend::value_variants() {
            assert_eq!(
                run_profile(&args, *backend),
                backend.lib_backend().default_development_profile(),
                "{backend:?} flag-free profile drifted from the backend default"
            );
        }
        assert_eq!(
            run_profile(&args, TargetBackend::Hydrolysis),
            BuildProfile::Optimized
        );
        assert_eq!(
            run_profile(&args, TargetBackend::Apple),
            BuildProfile::Debug
        );
    }

    #[test]
    fn run_profile_flags_override_the_default() {
        assert_eq!(
            run_profile(&run_args(&["--debug"]), TargetBackend::Hydrolysis),
            BuildProfile::Debug
        );
        assert_eq!(
            run_profile(&run_args(&["--release"]), TargetBackend::Hydrolysis),
            BuildProfile::Release
        );
        assert_eq!(
            run_profile(&run_args(&["--profiling"]), TargetBackend::Hydrolysis),
            BuildProfile::Profiling
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
    fn env_assignment_splits_on_first_equals() {
        assert_eq!(
            parse_env_assignment("A=b=c").expect("value may hold '='"),
            (String::from("A"), String::from("b=c"))
        );
        assert_eq!(
            parse_env_assignment("EMPTY=").expect("empty value is fine"),
            (String::from("EMPTY"), String::new())
        );
    }

    #[test]
    fn env_assignment_rejects_malformed_pairs() {
        assert!(parse_env_assignment("NOEQUALS").is_err());
        assert!(parse_env_assignment("=novalue").is_err());
    }

    #[test]
    fn rejects_device_with_desktop_backend() {
        let err = validate_device_arg(TargetPlatform::Linux, TargetBackend::Gtk4, Some("foo"))
            .expect_err("gtk4 with --device should fail");
        assert!(err.to_string().contains("--device is not supported"));
        let err = validate_device_arg(
            TargetPlatform::Linux,
            TargetBackend::Hydrolysis,
            Some("foo"),
        )
        .expect_err("hydrolysis with --device should fail");
        assert!(err.to_string().contains("--device is not supported"));
    }

    #[test]
    fn accepts_device_with_non_gtk4_backend() {
        assert!(
            validate_device_arg(TargetPlatform::Ios, TargetBackend::Apple, Some("sim-1")).is_ok()
        );
    }

    #[test]
    fn rejects_device_on_local_only_targets() {
        // The Apple backend on macOS has exactly one device — this machine —
        // so --device can only be a mistake.
        let err = validate_device_arg(TargetPlatform::Macos, TargetBackend::Apple, Some("foo"))
            .expect_err("apple/macos with --device should fail");
        assert!(err.to_string().contains("--device is not supported"));
        let err = validate_device_arg(TargetPlatform::Web, TargetBackend::Hydrolysis, Some("foo"))
            .expect_err("web with --device should fail");
        assert!(err.to_string().contains("--device is not supported"));
    }

    #[test]
    fn device_choice_prefers_remembered_device() {
        let candidates = vec![
            device_candidate("sim-1"),
            device_candidate("phys-1"),
            device_candidate("sim-2"),
        ];
        match device_choice(&candidates, Some("phys-1")) {
            DeviceChoice::Use(1, false) => {}
            other => panic!("expected Use(1, false), got {other:?}"),
        }
    }

    #[test]
    fn device_choice_reports_stale_memory() {
        let candidates = vec![device_candidate("sim-1")];
        match device_choice(&candidates, Some("gone")) {
            DeviceChoice::Stale(id) => assert_eq!(id, "gone"),
            other => panic!("expected Stale, got {other:?}"),
        }
    }

    #[test]
    fn device_choice_single_candidate_is_unambiguous() {
        let candidates = vec![device_candidate("sim-1")];
        match device_choice(&candidates, None) {
            DeviceChoice::Use(0, false) => {}
            other => panic!("expected Use(0, false), got {other:?}"),
        }
    }

    #[test]
    fn device_choice_multiple_candidates_prompts() {
        let candidates = vec![device_candidate("a"), device_candidate("b")];
        assert!(matches!(
            device_choice(&candidates, None),
            DeviceChoice::Prompt
        ));
    }

    #[test]
    fn non_interactive_multi_device_error_lists_candidates() {
        let shell = crate::shell::Shell::new(true);
        let candidates = vec![device_candidate("serial-a"), device_candidate("avd-b")];
        let err = prompt_for_device(&shell, "Pick", &candidates, None)
            .expect_err("non-interactive runs cannot pick");
        let message = err.to_string();
        assert!(message.contains("--device"));
        assert!(message.contains("serial-a") && message.contains("avd-b"));
    }

    fn device_candidate(id: &str) -> DeviceCandidate {
        DeviceCandidate {
            device: SelectedDevice::Local(Local),
            id: id.to_owned(),
            label: id.to_owned(),
        }
    }

    #[test]
    fn clean_device_exit_stops_without_error() {
        let shell = crate::shell::Shell::new(false);
        handle_device_event(
            &shell,
            DeviceEvent::Exited(ApplicationExit::completed()),
            "test",
        )
        .expect("clean device exit should not fail water run");
    }

    fn monitored_stop_result(
        report_monitor_error: bool,
    ) -> (eyre::Result<()>, Option<waterui_cli::device::StopRequest>) {
        let shell = crate::shell::Shell::new(false);
        let (running, sender, control) = waterui_cli::device::Running::new();
        let (request_tx, request_rx) = smol::channel::bounded(1);
        let (interrupt_sender, interrupts) = smol::channel::unbounded();
        interrupt_sender
            .try_send(())
            .expect("queue the stop interrupt");

        smol::spawn(async move {
            let request = control.recv().await.ok();
            let _ = request_tx.try_send(request);
            if report_monitor_error {
                sender
                    .send(DeviceEvent::MonitorError {
                        message:
                            "The app did not exit within the 5s termination grace period; killing it"
                                .to_string(),
                    })
                    .await
                    .expect("send the monitor error");
            }
            sender
                .send(DeviceEvent::Exited(ApplicationExit::user_closed()))
                .await
                .expect("send the terminal event");
        })
        .detach();

        let backend = TargetBackend::Hydrolysis;
        #[cfg(target_os = "macos")]
        let result = {
            let host = waterui_cli::toolchain::Host::current();
            let mut crash_ctx = None;
            smol::block_on(stream_running_events(
                &shell,
                &host,
                running,
                interrupts,
                backend,
                &mut crash_ctx,
            ))
        };
        #[cfg(not(target_os = "macos"))]
        let result = smol::block_on(stream_running_events(&shell, running, interrupts, backend));

        let request = request_rx.try_recv().ok().flatten();
        (result, request)
    }

    #[test]
    fn stop_after_a_monitor_error_fails_the_run() {
        let (result, request) = monitored_stop_result(true);
        assert_eq!(request, Some(StopRequest::Terminate));
        let error = result.expect_err("a stop that reported an error fails water run");
        assert_eq!(error.to_string(), "the run's monitor reported 1 error(s)");
    }

    #[test]
    fn plain_stop_succeeds() {
        let (result, request) = monitored_stop_result(false);
        assert_eq!(request, Some(StopRequest::Terminate));
        result.expect("a plain stop exits 0");
    }

    #[test]
    fn android_abi_follows_the_device() {
        // The ABI is a property of the selected device: a desktop run
        // selects the local machine and resolves no ABI, whatever backend
        // the run uses — Hydrolysis included.
        assert_eq!(
            device_android_abi(&SelectedDevice::AndroidDevice(AndroidDevice::new(
                String::from("serial-1"),
                AndroidAbi::Arm64V8a,
            ))),
            Some(AndroidAbi::Arm64V8a)
        );
        assert_eq!(device_android_abi(&SelectedDevice::Local(Local)), None);
    }

    #[test]
    fn resolve_backend_defaults_include_web() {
        assert_eq!(
            resolve_backend(TargetPlatform::Web, None).expect("web backend"),
            TargetBackend::Hydrolysis
        );
    }

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
            TargetBackend::Hydrolysis
        );
        assert_eq!(
            resolve_backend(TargetPlatform::Windows, None).expect("windows backend"),
            TargetBackend::Hydrolysis
        );
    }

    #[test]
    fn default_backend_is_the_platforms_native_backend() {
        assert_eq!(default_backend(TargetPlatform::Macos), TargetBackend::Apple);
        assert_eq!(default_backend(TargetPlatform::Ios), TargetBackend::Apple);
        assert_eq!(
            default_backend(TargetPlatform::Android),
            TargetBackend::Android
        );
        assert_eq!(
            default_backend(TargetPlatform::Linux),
            TargetBackend::Hydrolysis
        );
        assert_eq!(
            default_backend(TargetPlatform::Windows),
            TargetBackend::Hydrolysis
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_platform_defaults_to_host_on_macos() {
        assert_eq!(resolve_platform(None), TargetPlatform::Macos);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolve_platform_defaults_to_host_on_linux() {
        assert_eq!(resolve_platform(None), TargetPlatform::Linux);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn resolve_platform_defaults_to_host_on_windows() {
        assert_eq!(resolve_platform(None), TargetPlatform::Windows);
    }

    #[test]
    fn desktop_platform_must_match_the_host() {
        // `--platform linux` resolves host-native (`Triple::host()`), so on a
        // macOS host it would build a darwin binary — reject it instead.
        let err = waterui_cli::platform::ensure_desktop_platform_is_host(
            TargetPlatform::Linux.desktop_os(),
            "macos",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "`--platform linux` targets the host; this host is macos."
        );
        assert!(
            waterui_cli::platform::ensure_desktop_platform_is_host(
                TargetPlatform::Linux.desktop_os(),
                "linux"
            )
            .is_ok()
        );
        // Device, web and embedded platforms carry their own triples.
        for platform in [
            TargetPlatform::Ios,
            TargetPlatform::Android,
            TargetPlatform::Web,
            TargetPlatform::Esp32s3,
        ] {
            assert!(
                waterui_cli::platform::ensure_desktop_platform_is_host(
                    platform.desktop_os(),
                    "linux"
                )
                .is_ok()
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn desktop_backend_platform_must_match_macos_host() {
        assert!(
            TargetBackend::Gtk4
                .lib_backend()
                .validate_host_support(lib_platform(TargetPlatform::Macos))
                .is_err()
        );
        assert!(
            TargetBackend::Hydrolysis
                .lib_backend()
                .validate_host_support(lib_platform(TargetPlatform::Macos))
                .is_ok()
        );
        assert!(
            TargetBackend::Gtk4
                .lib_backend()
                .validate_host_support(lib_platform(TargetPlatform::Linux))
                .is_err()
        );
        assert!(
            TargetBackend::Hydrolysis
                .lib_backend()
                .validate_host_support(lib_platform(TargetPlatform::Linux))
                .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn desktop_backend_platform_must_match_linux_host() {
        assert!(
            TargetBackend::Gtk4
                .lib_backend()
                .validate_host_support(lib_platform(TargetPlatform::Linux))
                .is_ok()
        );
        assert!(
            TargetBackend::Hydrolysis
                .lib_backend()
                .validate_host_support(lib_platform(TargetPlatform::Linux))
                .is_ok()
        );
        assert!(
            TargetBackend::Gtk4
                .lib_backend()
                .validate_host_support(lib_platform(TargetPlatform::Macos))
                .is_err()
        );
        assert!(
            TargetBackend::Hydrolysis
                .lib_backend()
                .validate_host_support(lib_platform(TargetPlatform::Macos))
                .is_err()
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn desktop_backend_platform_must_match_windows_host() {
        assert!(
            TargetBackend::Hydrolysis
                .lib_backend()
                .validate_host_support(lib_platform(TargetPlatform::Windows))
                .is_ok()
        );
        assert!(
            TargetBackend::Gtk4
                .lib_backend()
                .validate_host_support(lib_platform(TargetPlatform::Windows))
                .is_err()
        );
        assert!(
            TargetBackend::Hydrolysis
                .lib_backend()
                .validate_host_support(lib_platform(TargetPlatform::Macos))
                .is_err()
        );
    }
}
