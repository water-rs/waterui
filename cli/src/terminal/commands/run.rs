//! `water run` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use eyre::{Context as _, Result, bail};
use futures_util::StreamExt;

#[cfg(target_os = "macos")]
use jiff::Timestamp;

use super::detect_sccache_path;
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
    backend::reinit_backend,
    build::BuildOptions,
    device::{Artifact, Device, DeviceEvent, Local, LogLevel, RunOptions, Running},
    esp32::{backend::Esp32Backend, platform::run_esp32},
    gtk4::{
        backend::Gtk4Backend,
        platform::{build_gtk4, package_gtk4},
    },
    hydrolysis::{
        backend::HydrolysisBackend,
        platform::{
            HydrolysisWebDevServer, build_hydrolysis, package_hydrolysis,
            prepare_hydrolysis_web_dev_site,
        },
    },
    platform::{PackageOptions, TargetPlatform as LibTargetPlatform},
    project::Project,
    web,
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
        // Xcode product name — the same one the bundle is built under.
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

#[derive(Debug, Clone, Copy)]
struct BackendAvailability {
    available: [bool; 5],
}

impl BackendAvailability {
    const fn has(self, backend: TargetBackend) -> bool {
        self.available[match backend {
            TargetBackend::Apple => 0,
            TargetBackend::Android => 1,
            TargetBackend::Gtk4 => 2,
            TargetBackend::Hydrolysis => 3,
            TargetBackend::Dew => 4,
        }]
    }
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
}

/// Target backend for running (how the app is built and rendered).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TargetBackend {
    /// Apple backend (UIKit/AppKit).
    Apple,
    /// Android backend (Android Views).
    Android,
    /// GTK4 backend (Linux only).
    Gtk4,
    /// Hydrolysis backend (self-drawn renderer).
    Hydrolysis,
    /// Dew backend (ESP32 firmware).
    Dew,
}

/// Arguments for the run command.
#[derive(ClapArgs, Debug)]
// CLI flag structs collect booleans by nature; each flag is a documented
// `--flag`, not structural state.
#[allow(clippy::struct_excessive_bools)]
pub struct Args {
    /// Target platform to run on.
    /// Defaults to the host platform when omitted.
    #[arg(short, long, value_enum)]
    platform: Option<TargetPlatform>,

    /// Backend to use (overrides default for platform).
    /// Example: `--platform linux --backend hydrolysis`.
    #[arg(short, long, value_enum)]
    backend: Option<TargetBackend>,

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

    /// Do not start the frontend dev server for `include_web!` mounts; the
    /// debug build renders the staged bundle, packaged as `water package`
    /// does.
    #[arg(long)]
    no_dev_server: bool,
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
        TargetPlatform::Linux => TargetBackend::Gtk4,
        TargetPlatform::Windows | TargetPlatform::Web => TargetBackend::Hydrolysis,
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
            | (TargetPlatform::Android, TargetBackend::Android)
            | (
                TargetPlatform::Linux,
                TargetBackend::Gtk4 | TargetBackend::Hydrolysis
            )
            | (
                TargetPlatform::Windows | TargetPlatform::Web,
                TargetBackend::Hydrolysis
            )
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
             - Android: android\n  \
             - Linux: gtk4, hydrolysis\n  \
             - Windows: hydrolysis\n  \
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

const fn default_backend_priority(platform: TargetPlatform) -> &'static [TargetBackend] {
    match platform {
        TargetPlatform::Ios => &[TargetBackend::Apple],
        TargetPlatform::Android => &[TargetBackend::Android],
        TargetPlatform::Macos => &[TargetBackend::Apple, TargetBackend::Hydrolysis],
        TargetPlatform::Linux => &[TargetBackend::Gtk4, TargetBackend::Hydrolysis],
        TargetPlatform::Windows | TargetPlatform::Web => &[TargetBackend::Hydrolysis],
        TargetPlatform::Esp32s3 | TargetPlatform::Esp32c3 | TargetPlatform::Esp32p4 => {
            &[TargetBackend::Dew]
        }
    }
}

fn resolve_default_backend_for_project(
    platform: TargetPlatform,
    project_is_playground: bool,
    availability: BackendAvailability,
) -> TargetBackend {
    let backends = default_backend_priority(platform);
    if project_is_playground {
        return backends[0];
    }

    for backend in backends {
        if availability.has(*backend) {
            return *backend;
        }
    }

    backends[0]
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
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    if args.tui {
        return run_tui_app(shell, args).await;
    }

    let host = waterui_cli::toolchain::Host::current();
    // The run context carries the opened project, the resolved device, backend
    // and build options; on Windows that future crosses clippy's `large_futures`
    // threshold (16 KiB), so it is pinned on the heap instead of the caller's stack.
    let context = Box::pin(prepare_run_context(shell, &args)).await?;
    print_run_header(shell, &context);
    check_run_toolchain(shell, &host, context.platform, context.backend).await?;

    if context.platform == TargetPlatform::Web {
        return run_web_app(shell, &context.project).await;
    }

    if context.platform.esp32_chip().is_some() {
        return run_esp32_app(shell, &context.project, args.device.as_deref()).await;
    }

    let selection = select_run_device(
        shell,
        &host,
        context.platform,
        context.backend,
        &context.project,
        args.device.as_deref(),
    )
    .await?;
    let config = build_run_config(shell, &host, &args, &context.project).await;

    #[cfg(target_os = "macos")]
    let mut crash_ctx =
        match CrashReportContext::try_new(&context.project, context.platform, context.backend) {
            Ok(ctx) => ctx,
            Err(e) => {
                warn!(shell, "Crash report augmentation disabled: {e}");
                None
            }
        };

    // The dev-server guard is held for the app's whole run: dropping it —
    // on app exit, normal return, or the Ctrl-C future-drop — kills the
    // bundler child (`kill_on_drop`).
    let (running, _dev_server) = shell
        .display_output(build_and_run(
            shell,
            &host,
            &context.project,
            context.platform,
            context.backend,
            selection,
            config,
        ))
        .await?;

    line!(shell);
    note!(shell, "Press Ctrl+C to stop the application");
    line!(shell);

    // Stream device events
    #[cfg(target_os = "macos")]
    stream_running_events(shell, &host, running, context.backend, &mut crash_ctx).await?;
    #[cfg(not(target_os = "macos"))]
    stream_running_events(shell, running, context.backend).await?;

    Ok(())
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

    let project_path = crate::project_path::canonicalize(&args.path)?;
    let project = Project::open(&project_path).await?;
    let launcher_dir = waterui_cli::tui::ensure_launcher(&project).await?;

    let sccache_path = detect_sccache_path(shell, &waterui_cli::toolchain::Host::current()).await;
    let binary = shell
        .display_output(waterui_cli::tui::build(
            &project,
            &launcher_dir,
            sccache_path,
        ))
        .await?;

    note!(
        shell,
        "The TUI backend replaces this terminal until the app exits"
    );
    shell.clear();
    waterui_cli::tui::exec(&binary)
}

async fn prepare_run_context(shell: &Shell, args: &Args) -> Result<RunContext> {
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let mut project = Project::open(&project_path).await?;
    let platform = resolve_platform(args.platform);
    let backend = resolve_run_backend(&project, platform, args.backend)?;

    validate_desktop_backend_platform_on_host(platform, backend)?;
    validate_device_arg(platform, backend, args.device.as_deref())?;
    validate_log_pipeline_args(platform, args.logs, args.native_logs)?;
    ensure_run_backend_ready(&project, backend)?;

    // Selecting an ESP32 platform pins the chip so the generated harness and
    // build target follow the platform (the chip drives the target triple,
    // QEMU model, and firmware parameters).
    if let Some(chip) = platform.esp32_chip() {
        project.set_esp32_chip(chip).await?;
    }

    let project = ensure_generated_run_backend(shell, &project_path, project, backend).await?;

    Ok(RunContext {
        project,
        platform,
        backend,
    })
}

fn resolve_run_backend(
    project: &Project,
    platform: TargetPlatform,
    backend_override: Option<TargetBackend>,
) -> Result<TargetBackend> {
    resolve_backend(
        platform,
        backend_override.or_else(|| {
            Some(resolve_default_backend_for_project(
                platform,
                project.is_playground(),
                backend_availability(project),
            ))
        }),
    )
}

const fn backend_availability(project: &Project) -> BackendAvailability {
    BackendAvailability {
        available: [
            project.apple_backend().is_some(),
            project.android_backend().is_some(),
            project.gtk4_backend().is_some(),
            project.hydrolysis_backend().is_some(),
            project.esp32_backend().is_some(),
        ],
    }
}

fn ensure_run_backend_ready(project: &Project, backend: TargetBackend) -> Result<()> {
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
        TargetBackend::Dew if project.esp32_backend().is_none() => {
            bail!("ESP32 backend is not configured. Run `water backend add esp32`.");
        }
        _ => Ok(()),
    }
}

async fn ensure_generated_run_backend(
    shell: &Shell,
    project_path: &PathBuf,
    project: Project,
    backend: TargetBackend,
) -> Result<Project> {
    match backend {
        TargetBackend::Gtk4 if project.is_playground() => {
            let needs_reinit = Gtk4Backend::requires_regeneration(&project).await?;
            ensure_generated_run_backend_impl::<Gtk4Backend>(
                shell,
                project_path,
                project,
                needs_reinit,
                "Initializing GTK4 backend...",
                "GTK4 backend initialized",
            )
            .await
        }
        TargetBackend::Hydrolysis if project.is_playground() => {
            let needs_reinit = HydrolysisBackend::requires_regeneration(&project).await?;
            ensure_generated_run_backend_impl::<HydrolysisBackend>(
                shell,
                project_path,
                project,
                needs_reinit,
                "Initializing hydrolysis backend...",
                "Hydrolysis backend initialized",
            )
            .await
        }
        TargetBackend::Dew if project.is_playground() => {
            let needs_reinit =
                project.esp32_backend().is_none() || Esp32Backend::requires_regeneration(&project)?;
            ensure_generated_run_backend_impl::<Esp32Backend>(
                shell,
                project_path,
                project,
                needs_reinit,
                "Initializing ESP32 backend...",
                "ESP32 backend initialized",
            )
            .await
        }
        _ => Ok(project),
    }
}

async fn ensure_generated_run_backend_impl<T>(
    shell: &Shell,
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

    let spinner = shell.spinner(spinner_message);
    reinit_backend::<T>(&project).await?;
    let project = Project::open(project_path).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!(shell, "{success_message}");
    Ok(project)
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
    let build_options = sccache_path.map_or_else(
        || BuildOptions::development(false),
        |sccache| BuildOptions::development(false).with_sccache(sccache),
    );

    let _ = shell.status(">", "Building ESP32 firmware...");
    shell
        .display_output(run_esp32(project, build_options, device))
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
    let spinner = shell.spinner("Scanning for devices...");
    let device = find_device(host, platform, backend, project, device_id).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

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

    BuildRunConfig {
        run_options,
        sccache_path,
        release: args.release,
        dev_server: !args.release && !args.no_dev_server,
    }
}

#[cfg(target_os = "macos")]
async fn stream_running_events(
    shell: &Shell,
    host: &waterui_cli::toolchain::Host,
    running: Running,
    backend: TargetBackend,
    crash_ctx: &mut Option<CrashReportContext>,
) -> Result<()> {
    let mut running = std::pin::pin!(running);
    let backend_log_name = backend_name(backend);

    loop {
        let event = running.next().await;

        #[cfg(target_os = "macos")]
        let mut event = event;

        #[cfg(target_os = "macos")]
        if matches!(event.as_ref(), Some(DeviceEvent::Started))
            && let Some(ctx) = crash_ctx.as_mut()
        {
            ctx.refresh_start();
        }

        #[cfg(target_os = "macos")]
        if let Some(ctx) = crash_ctx.as_ref() {
            event = augment_event_with_crash_report(host, event, ctx).await;
        }

        if handle_device_event(shell, event, backend_log_name)? {
            break;
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn stream_running_events(
    shell: &Shell,
    running: Running,
    backend: TargetBackend,
) -> Result<()> {
    let mut running = std::pin::pin!(running);
    let backend_log_name = backend_name(backend);

    loop {
        if handle_device_event(shell, running.next().await, backend_log_name)? {
            break;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn augment_event_with_crash_report(
    host: &waterui_cli::toolchain::Host,
    event: Option<DeviceEvent>,
    ctx: &CrashReportContext,
) -> Option<DeviceEvent> {
    use std::fmt::Write as _;

    match event {
        Some(DeviceEvent::Exited(exit)) => {
            if let Some(report) = find_latest_ips_report(host, ctx).await {
                return Some(DeviceEvent::Crashed(report.to_string()));
            }
            Some(DeviceEvent::Exited(exit))
        }
        Some(DeviceEvent::Crashed(mut msg)) => {
            if !msg.contains("Crash report:")
                && let Some(report) = find_latest_ips_report(host, ctx).await
            {
                write!(msg, "\n\nCrash report: {}", report.log_path().display())
                    .expect("write to String");
            }
            Some(DeviceEvent::Crashed(msg))
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
    let build_plan = resolve_build_plan(cli_platform, backend, &selection.device)?;
    let physical_ios = is_physical_ios(&selection.device);
    let launch_task =
        spawn_device_launch_task(host.clone(), selection.device, selection.needs_launch);

    let _ = shell.status(">", "Building...");
    build_for_backend(project, backend, &build_plan, build_options(&config)).await?;

    // A declared `include_web!` mount is served by the bundler's own dev
    // server in debug runs — spawn it after the Rust build (its root comes
    // from the compiled metadata) and before packaging, so the packaged app
    // stages no web output.
    let dev_server = if config.dev_server {
        start_web_dev_server(shell, project, config.sccache_path.as_deref(), physical_ios).await?
    } else {
        None
    };

    let _ = shell.status(">", "Packaging...");
    let artifact = package_for_backend(
        project,
        backend,
        &build_plan,
        config.release,
        dev_server.is_some(),
    )
    .await?;

    if selection.needs_launch {
        let _ = shell.status(">", "Waiting for device...");
    }
    let device = launch_task.await?;

    let mut run_options = config.run_options;
    if let Some(server) = &dev_server {
        apply_dev_url_handoff(&device, server.url(), &mut run_options)?;
    }

    let _ = shell.status(">", "Running...");
    let running = run_with_options(host, device, artifact, run_options).await?;

    Ok((running, dev_server))
}

/// Spawn the declared frontend's dev server for a debug run, when the
/// compiled library mounts one with `include_web!`.
async fn start_web_dev_server(
    shell: &Shell,
    project: &Project,
    sccache_path: Option<&std::path::Path>,
    expose_on_lan: bool,
) -> Result<Option<web::WebDevServer>> {
    let Some(meta) = web::web_mount(project, sccache_path).await? else {
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

fn resolve_build_plan(
    cli_platform: TargetPlatform,
    backend: TargetBackend,
    device: &SelectedDevice,
) -> Result<BuildPlan> {
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
    let android_abi = resolve_android_abi(backend, device)?;

    Ok(BuildPlan {
        lib_platform,
        android_abi,
    })
}

fn resolve_android_abi(
    backend: TargetBackend,
    device: &SelectedDevice,
) -> Result<Option<waterui_cli::android::platform::AndroidAbi>> {
    match (backend, device) {
        (TargetBackend::Android, SelectedDevice::AndroidDevice(dev)) => Ok(Some(dev.abi())),
        (TargetBackend::Android, SelectedDevice::AndroidEmulator(emu)) => {
            Ok(Some(emu.expected_abi()))
        }
        (TargetBackend::Android, _) => {
            bail!("Internal error: Android backend requires an Android device");
        }
        _ => Ok(None),
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
            || BuildOptions::development(config.release),
            |sccache| BuildOptions::development(config.release).with_sccache(sccache.clone()),
        )
        .with_dev_server(config.dev_server)
}

async fn build_for_backend(
    project: &Project,
    backend: TargetBackend,
    plan: &BuildPlan,
    build_options: BuildOptions,
) -> Result<()> {
    match backend {
        TargetBackend::Apple => {
            build_rust_lib(project, plan.lib_platform, build_options).await?;
        }
        TargetBackend::Android => {
            let abi = plan
                .android_abi
                .ok_or_else(|| eyre::eyre!("Internal error: missing Android ABI for build"))?;
            AndroidPlatform::clean_jni_libs(project).await?;
            AndroidPlatform::new(abi)
                .build(project, build_options)
                .await?;
        }
        TargetBackend::Gtk4 => {
            build_gtk4(project, build_options).await?;
        }
        TargetBackend::Hydrolysis => {
            build_hydrolysis(project, plan.lib_platform, build_options).await?;
        }
        TargetBackend::Dew => {
            panic!("esp32 run should not enter build_and_run")
        }
    }
    Ok(())
}

async fn package_for_backend(
    project: &Project,
    backend: TargetBackend,
    plan: &BuildPlan,
    release: bool,
    dev_server: bool,
) -> Result<Artifact> {
    let package_options = PackageOptions::development()
        .with_debug(!release)
        .with_dev_server(dev_server);
    match backend {
        TargetBackend::Apple => package_apple(project, plan.lib_platform, package_options).await,
        TargetBackend::Android => {
            let abi = plan
                .android_abi
                .ok_or_else(|| eyre::eyre!("Internal error: missing Android ABI for packaging"))?;
            AndroidPlatform::package_with_abis(project, package_options, &[abi]).await
        }
        TargetBackend::Gtk4 => package_gtk4(project, package_options).await,
        TargetBackend::Hydrolysis => {
            package_hydrolysis(project, plan.lib_platform, package_options).await
        }
        TargetBackend::Dew => panic!("esp32 run should not enter build_and_run"),
    }
}

struct BuildRunConfig {
    run_options: RunOptions,
    sccache_path: Option<PathBuf>,
    release: bool,
    /// Whether a declared `include_web!` mount may be served by the bundler's
    /// dev server: debug builds only, unless `--no-dev-server` opts out.
    dev_server: bool,
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

/// Select the iOS target: an explicit `--device` may name a paired physical
/// device or a simulator; without one, simulator selection stays as it was
/// and a physical device is only picked up when no simulator qualifies.
async fn select_ios_device(
    host: &waterui_cli::toolchain::Host,
    project: &Project,
    device_id: Option<&str>,
) -> Result<SelectedDevice> {
    let physical = ApplePhysicalDevice::scan(host)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!("devicectl device scan failed: {error:#}");
            Vec::new()
        });

    if let Some(query) = device_id
        && let Some(device) = select_physical_ios(host, &physical, project, query).await?
    {
        return Ok(SelectedDevice::ApplePhysical(device));
    }

    match AppleSimulator::select_ios(host, project, device_id).await {
        Ok(sim) => Ok(SelectedDevice::AppleSimulator(sim)),
        Err(sim_error) => {
            // No explicit device and no qualifying simulator — fall back to a
            // usable physical device if one is paired.
            if device_id.is_some() {
                return Err(sim_error);
            }
            let Some(device) = usable_physical_ios(host, &physical, project).await? else {
                return Err(sim_error);
            };
            Ok(SelectedDevice::ApplePhysical(device))
        }
    }
}

/// The first paired device that can actually run the app: reachable, booted,
/// Developer Mode on, OS at or above the deployment target.
async fn usable_physical_ios(
    host: &waterui_cli::toolchain::Host,
    devices: &[ApplePhysicalDevice],
    project: &Project,
) -> Result<Option<ApplePhysicalDevice>> {
    let (_, target) =
        waterui_cli::apple::platform::apple_deployment_target(project, LibTargetPlatform::IOS)
            .await?;
    let deployment_target =
        waterui_cli::utils::parse_semver_version(&target).wrap_err_with(|| {
            format!("Failed to parse the project's IPHONEOS_DEPLOYMENT_TARGET `{target}`")
        })?;
    let Some(device) = devices.iter().find(|device| {
        device.usability().is_ok() && device.supports_deployment_target(&deployment_target)
    }) else {
        return Ok(None);
    };
    // A device build must be signed; check the keychain has a development
    // identity now rather than after a multi-minute build.
    waterui_cli::apple::toolchain::development_team_id(host).await?;
    Ok(Some(device.clone()))
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
            {
                bail!("Internal error: hydrolysis backend is not supported on {platform:?}");
            }
            if platform == TargetPlatform::Web {
                toolchain_checks::check_web(host).await?;
            } else {
                toolchain_checks::check_hydrolysis(host).await?;
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

async fn find_device(
    host: &waterui_cli::toolchain::Host,
    platform: TargetPlatform,
    backend: TargetBackend,
    project: &Project,
    device_id: Option<&str>,
) -> Result<SelectedDevice> {
    // For native desktop Rust backends, always use Local device regardless of platform.
    if backend == TargetBackend::Gtk4 || backend == TargetBackend::Hydrolysis {
        return Ok(SelectedDevice::Local(Local));
    }

    match platform {
        TargetPlatform::Ios => select_ios_device(host, project, device_id).await,
        TargetPlatform::Macos => {
            // macOS with Apple backend uses the local machine
            Ok(SelectedDevice::Local(Local))
        }
        TargetPlatform::Android => {
            let devices = AndroidDevice::scan(host).await?;

            if let Some(id) = device_id {
                // Find specific device
                for dev in devices {
                    if dev.identifier() == id {
                        return Ok(SelectedDevice::AndroidDevice(dev));
                    }
                }
                bail!("Device not found: {id}");
            }

            // If we have a connected device, use it
            if let Some(dev) = devices.into_iter().next() {
                return Ok(SelectedDevice::AndroidDevice(dev));
            }

            // No connected devices - try to find an emulator AVD
            let avds = AndroidPlatform::list_avds(host).await?;
            let avd_name = avds.into_iter().next().ok_or_else(|| {
                eyre::eyre!(
                    "No Android devices connected and no emulators available. Create an emulator with Android Studio or `avdmanager`, or connect a device."
                )
            })?;

            Ok(SelectedDevice::AndroidEmulator(
                AndroidEmulator::open(host, avd_name).await?,
            ))
        }
        TargetPlatform::Linux => {
            // Linux runs on the local machine
            Ok(SelectedDevice::Local(Local))
        }
        TargetPlatform::Windows => {
            // Windows runs on the local machine
            Ok(SelectedDevice::Local(Local))
        }
        TargetPlatform::Web => {
            bail!("web platform does not use the device pipeline");
        }
        TargetPlatform::Esp32s3 | TargetPlatform::Esp32c3 | TargetPlatform::Esp32p4 => {
            bail!("esp32 platform does not use the device pipeline");
        }
    }
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
        TargetPlatform::Ios => "iOS Simulator",
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
        TargetBackend::Dew => "Dew",
    }
}

fn validate_device_arg(
    platform: TargetPlatform,
    backend: TargetBackend,
    device: Option<&str>,
) -> Result<()> {
    if platform == TargetPlatform::Web && device.is_some() {
        bail!("--device is not supported with the web platform");
    }

    if matches!(backend, TargetBackend::Gtk4 | TargetBackend::Hydrolysis)
        && platform != TargetPlatform::Web
        && device.is_some()
    {
        bail!(
            "--device is not supported with desktop backends (gtk4/hydrolysis run on the local machine)"
        );
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
        // The Dew/ESP32 firmware cross-compiles from any host with espup installed.
        TargetBackend::Android | TargetBackend::Dew => {}
    }

    Ok(())
}

/// Handle a device event.
///
/// Returns `true` if the event loop should break.
fn handle_device_event(
    shell: &Shell,
    event: Option<DeviceEvent>,
    platform_name: &str,
) -> Result<bool> {
    match event {
        Some(DeviceEvent::Started) => {
            let _ = shell.status("*", "Application started");
            Ok(false)
        }
        Some(DeviceEvent::Stopped) => {
            let _ = shell.status("o", "Application stopped");
            Ok(true)
        }
        Some(DeviceEvent::Stdout { message }) => {
            line!(shell, "[stdout] {message}");
            Ok(false)
        }
        Some(DeviceEvent::Stderr { message }) => {
            warn!(shell, "[stderr] {message}");
            Ok(false)
        }
        Some(DeviceEvent::Log { level, message }) => {
            let _ = shell.device_log(platform_name, level, message);
            Ok(false)
        }
        Some(DeviceEvent::Exited(exit)) => {
            let _ = shell.status("o", exit.terminal_message());
            Ok(true)
        }
        Some(DeviceEvent::Crashed(msg)) => {
            // Use panic_report for panic messages, regular error for others
            if msg.starts_with("Panic:") {
                shell.panic_message(&msg);
            } else {
                error!(shell, "Application crashed: {msg}");
            }
            bail!("application crashed");
        }
        None => Ok(true),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BackendAvailability, TargetBackend, TargetPlatform, handle_device_event,
        parse_env_assignment, resolve_backend, resolve_default_backend_for_project,
        resolve_platform, validate_desktop_backend_platform_on_host, validate_device_arg,
    };
    use waterui_cli::device::{ApplicationExit, DeviceEvent};

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
    fn clean_device_exit_stops_without_error() {
        let shell = crate::shell::Shell::new(false);
        let should_stop = handle_device_event(
            &shell,
            Some(DeviceEvent::Exited(ApplicationExit::completed())),
            "test",
        )
        .expect("clean device exit should not fail water run");
        assert!(should_stop);
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
            TargetBackend::Gtk4
        );
        assert_eq!(
            resolve_backend(TargetPlatform::Windows, None).expect("windows backend"),
            TargetBackend::Hydrolysis
        );
    }

    #[test]
    fn default_backend_prefers_native_then_hydrolysis_for_app_projects() {
        assert_eq!(
            resolve_default_backend_for_project(
                TargetPlatform::Linux,
                false,
                BackendAvailability {
                    available: [false, false, false, true, false],
                }
            ),
            TargetBackend::Hydrolysis
        );
        assert_eq!(
            resolve_default_backend_for_project(
                TargetPlatform::Macos,
                false,
                BackendAvailability {
                    available: [false, false, false, true, false],
                }
            ),
            TargetBackend::Hydrolysis
        );
        assert_eq!(
            resolve_default_backend_for_project(
                TargetPlatform::Linux,
                false,
                BackendAvailability {
                    available: [false, false, false, false, false],
                }
            ),
            TargetBackend::Gtk4
        );
    }

    #[test]
    fn playground_defaults_use_platform_native_backend() {
        assert_eq!(
            resolve_default_backend_for_project(
                TargetPlatform::Macos,
                true,
                BackendAvailability {
                    available: [false, false, false, true, false],
                }
            ),
            TargetBackend::Apple
        );
        assert_eq!(
            resolve_default_backend_for_project(
                TargetPlatform::Linux,
                true,
                BackendAvailability {
                    available: [false, false, false, true, false],
                }
            ),
            TargetBackend::Gtk4
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

    #[cfg(target_os = "macos")]
    #[test]
    fn desktop_backend_platform_must_match_macos_host() {
        assert!(
            validate_desktop_backend_platform_on_host(TargetPlatform::Macos, TargetBackend::Gtk4)
                .is_err()
        );
        assert!(
            validate_desktop_backend_platform_on_host(
                TargetPlatform::Macos,
                TargetBackend::Hydrolysis
            )
            .is_ok()
        );
        assert!(
            validate_desktop_backend_platform_on_host(TargetPlatform::Linux, TargetBackend::Gtk4)
                .is_err()
        );
        assert!(
            validate_desktop_backend_platform_on_host(
                TargetPlatform::Linux,
                TargetBackend::Hydrolysis
            )
            .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn desktop_backend_platform_must_match_linux_host() {
        assert!(
            validate_desktop_backend_platform_on_host(TargetPlatform::Linux, TargetBackend::Gtk4)
                .is_ok()
        );
        assert!(
            validate_desktop_backend_platform_on_host(
                TargetPlatform::Linux,
                TargetBackend::Hydrolysis
            )
            .is_ok()
        );
        assert!(
            validate_desktop_backend_platform_on_host(TargetPlatform::Macos, TargetBackend::Gtk4)
                .is_err()
        );
        assert!(
            validate_desktop_backend_platform_on_host(
                TargetPlatform::Macos,
                TargetBackend::Hydrolysis
            )
            .is_err()
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn desktop_backend_platform_must_match_windows_host() {
        assert!(
            validate_desktop_backend_platform_on_host(
                TargetPlatform::Windows,
                TargetBackend::Hydrolysis
            )
            .is_ok()
        );
        assert!(
            validate_desktop_backend_platform_on_host(TargetPlatform::Windows, TargetBackend::Gtk4)
                .is_err()
        );
        assert!(
            validate_desktop_backend_platform_on_host(
                TargetPlatform::Macos,
                TargetBackend::Hydrolysis
            )
            .is_err()
        );
    }
}
