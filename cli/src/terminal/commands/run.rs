//! `water run` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};
use futures::StreamExt;

#[cfg(target_os = "macos")]
use jiff::Timestamp;

use crate::shell::{self, display_output};
use crate::toolchain_checks;
use crate::{error, header, line, note, success, warn};
use waterui_cli::{
    android::{
        device::{AndroidDevice, AndroidEmulator},
        platform::AndroidPlatform,
    },
    apple::{
        device::AppleSimulator,
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
    toolchain::sccache::Sccache,
    utils::sccache_install_hint,
};

#[cfg(target_os = "macos")]
use waterui_cli::debug;
#[cfg(target_os = "macos")]
use waterui_cli::project::PackageType;

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

        let device_identifier = whoami::fallible::hostname()
            .map_err(|e| color_eyre::eyre::eyre!("Failed to determine hostname: {e}"))?;

        let process_name = match project.manifest().package.package_type {
            PackageType::Playground => "WaterUIApp".to_string(),
            PackageType::App => {
                // Match Apple backend naming: convert crate name to UpperCamel for app name.
                project
                    .crate_name()
                    .split('-')
                    .map(|s| {
                        let mut chars = s.chars();
                        chars.next().map_or_else(String::new, |first| {
                            first.to_uppercase().chain(chars).collect()
                        })
                    })
                    .collect::<String>()
            }
        };

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
async fn find_latest_ips_report(ctx: &CrashReportContext) -> Option<debug::CrashReport> {
    debug::find_macos_ips_crash_report_since(
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
    /// ESP32-S3 board or QEMU (Dew firmware).
    Esp32s3,
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
    /// Streams device logs at or above this level.
    #[arg(long, value_enum)]
    logs: Option<CliLogLevel>,

    /// Include all native platform logs (`NSLog`, `print`, etc.), not just `WaterUI` logs.
    /// This is noisy but useful for debugging native code issues.
    #[arg(long)]
    native_logs: bool,
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
        TargetPlatform::Esp32s3 => TargetBackend::Dew,
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
            | (TargetPlatform::Esp32s3, TargetBackend::Dew)
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
             - ESP32-S3: dew",
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
        TargetPlatform::Esp32s3 => &[TargetBackend::Dew],
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

fn sccache_allowed() -> bool {
    if let Some(value) = std::env::var_os("WATERUI_DISABLE_SCCACHE") {
        let value = value.to_string_lossy().trim().to_ascii_lowercase();
        if matches!(value.as_str(), "1" | "true" | "yes" | "on") {
            return false;
        }
    }
    // Respect explicit wrapper from caller (e.g. passthrough wrapper in constrained envs).
    if std::env::var_os("RUSTC_WRAPPER").is_some() {
        return false;
    }
    true
}

/// Run the run command.
pub async fn run(args: Args) -> Result<()> {
    let context = prepare_run_context(&args).await?;
    print_run_header(&context);
    check_run_toolchain(context.platform, context.backend).await?;

    if context.platform == TargetPlatform::Web {
        return run_web_app(&context.project).await;
    }

    if context.platform == TargetPlatform::Esp32s3 {
        return run_esp32_app(&context.project, args.device.as_deref()).await;
    }

    let selection =
        select_run_device(context.platform, context.backend, args.device.as_deref()).await?;
    let config = build_run_config(&args).await;

    #[cfg(target_os = "macos")]
    let mut crash_ctx =
        match CrashReportContext::try_new(&context.project, context.platform, context.backend) {
            Ok(ctx) => ctx,
            Err(e) => {
                warn!("Crash report augmentation disabled: {e}");
                None
            }
        };

    let running = display_output(build_and_run(
        &context.project,
        context.platform,
        context.backend,
        selection.device,
        selection.needs_launch,
        config,
    ))
    .await?;

    line!();
    note!("Press Ctrl+C to stop the application");
    line!();

    // Stream device events
    #[cfg(target_os = "macos")]
    stream_running_events(running, context.backend, &mut crash_ctx).await?;
    #[cfg(not(target_os = "macos"))]
    stream_running_events(running, context.backend).await?;

    Ok(())
}

async fn prepare_run_context(args: &Args) -> Result<RunContext> {
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let project = Project::open(&project_path).await?;
    let platform = resolve_platform(args.platform);
    let backend = resolve_run_backend(&project, platform, args.backend)?;

    validate_desktop_backend_platform_on_host(platform, backend)?;
    validate_device_arg(platform, backend, args.device.as_deref())?;
    validate_log_pipeline_args(platform, args.logs, args.native_logs)?;
    ensure_run_backend_ready(&project, backend)?;
    let project = ensure_generated_run_backend(&project_path, project, backend).await?;

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
        TargetBackend::Dew if project.esp32_backend().is_none() => {
            bail!("ESP32 backend is not configured. Run `water backend add esp32`.")
        }
        _ => Ok(()),
    }
}

async fn ensure_generated_run_backend(
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
            ensure_generated_run_backend_impl::<Gtk4Backend>(
                project_path,
                project,
                needs_reinit,
                "Initializing GTK4 backend...",
                "GTK4 backend initialized",
            )
            .await
        }
        TargetBackend::Hydrolysis if project.is_playground() => {
            let needs_reinit = project.hydrolysis_backend().is_none()
                || HydrolysisBackend::requires_regeneration(&project)?;
            ensure_generated_run_backend_impl::<HydrolysisBackend>(
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

fn print_run_header(context: &RunContext) {
    header!(
        "Running {} on {} ({})",
        context.project.crate_name(),
        platform_name(context.platform),
        backend_name(context.backend)
    );
}

async fn check_run_toolchain(platform: TargetPlatform, backend: TargetBackend) -> Result<()> {
    let spinner = shell::spinner("Checking toolchain...");
    check_toolchain_for_backend(platform, backend).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Toolchain ready");
    Ok(())
}

async fn run_web_app(project: &Project) -> Result<()> {
    let spinner = shell::spinner("Building Hydrolysis web app...");
    let site_root = prepare_hydrolysis_web_dev_site(project).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Built Hydrolysis web app at {}", site_root.display());

    let server = HydrolysisWebDevServer::start(site_root).await?;
    line!();
    note!("Serving at http://{}/", server.address());
    note!("Press Ctrl+C to stop the web server");
    let _server = server;
    futures::future::pending::<()>().await;
    unreachable!("web dev server future should be cancelled by Ctrl+C")
}

async fn run_esp32_app(project: &Project, device: Option<&str>) -> Result<()> {
    let sccache_path = detect_sccache_path().await;
    let build_options = sccache_path.map_or_else(
        || BuildOptions::new(false),
        |sccache| BuildOptions::new(false).with_sccache(sccache),
    );

    shell::status(">", "Building ESP32 firmware...");
    display_output(run_esp32(project, build_options, device)).await
}

async fn select_run_device(
    platform: TargetPlatform,
    backend: TargetBackend,
    device_id: Option<&str>,
) -> Result<DeviceSelection> {
    let spinner = shell::spinner("Scanning for devices...");
    let device = find_device(platform, backend, device_id).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    let needs_launch = device.needs_launch();
    if needs_launch {
        note!("Will launch: {}", device_name(&device));
    } else {
        success!("Found device: {}", device_name(&device));
    }

    Ok(DeviceSelection {
        device,
        needs_launch,
    })
}

async fn build_run_config(args: &Args) -> BuildRunConfig {
    let sccache_path = detect_sccache_path().await;
    let mut run_options = RunOptions::new();
    if let Some(level) = args.logs.map(LogLevel::from) {
        run_options.set_log_level(level);
    }
    run_options.set_native_logs(args.native_logs);

    BuildRunConfig {
        run_options,
        sccache_path,
    }
}

async fn detect_sccache_path() -> Option<PathBuf> {
    if !sccache_allowed() {
        note!("Skipping sccache (explicit wrapper or WATERUI_DISABLE_SCCACHE is set)");
        return None;
    }

    let sccache = Sccache;
    sccache.path().await.map_or_else(
        |_| {
            warn!(
                "sccache not found. Build efficiency may be reduced. Install with: {}",
                sccache_install_hint()
            );
            None
        },
        Some,
    )
}

#[cfg(target_os = "macos")]
async fn stream_running_events(
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
            event = augment_event_with_crash_report(event, ctx).await;
        }

        if handle_device_event(event, backend_log_name)? {
            break;
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn stream_running_events(running: Running, backend: TargetBackend) -> Result<()> {
    let mut running = std::pin::pin!(running);
    let backend_log_name = backend_name(backend);

    loop {
        if handle_device_event(running.next().await, backend_log_name)? {
            break;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn augment_event_with_crash_report(
    event: Option<DeviceEvent>,
    ctx: &CrashReportContext,
) -> Option<DeviceEvent> {
    use std::fmt::Write as _;

    match event {
        Some(DeviceEvent::Exited(exit)) => {
            if let Some(report) = find_latest_ips_report(ctx).await {
                return Some(DeviceEvent::Crashed(report.to_string()));
            }
            Some(DeviceEvent::Exited(exit))
        }
        Some(DeviceEvent::Crashed(mut msg)) => {
            if !msg.contains("Crash report:")
                && let Some(report) = find_latest_ips_report(ctx).await
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
    project: &Project,
    cli_platform: TargetPlatform,
    backend: TargetBackend,
    device: SelectedDevice,
    needs_launch: bool,
    config: BuildRunConfig,
) -> Result<Running> {
    let build_plan = resolve_build_plan(cli_platform, backend, &device)?;
    let launch_task = spawn_device_launch_task(device, needs_launch);

    shell::status(">", "Building...");
    build_for_backend(project, backend, &build_plan, build_options(&config)).await?;

    shell::status(">", "Packaging...");
    let artifact = package_for_backend(project, backend, &build_plan).await?;

    if needs_launch {
        shell::status(">", "Waiting for device...");
    }
    let device = launch_task.await?;

    shell::status(">", "Running...");
    let running = run_with_options(device, artifact, config.run_options).await?;

    Ok(running)
}

fn resolve_build_plan(
    cli_platform: TargetPlatform,
    backend: TargetBackend,
    device: &SelectedDevice,
) -> Result<BuildPlan> {
    let lib_platform = match cli_platform {
        TargetPlatform::Ios => LibTargetPlatform::IOSSimulator,
        TargetPlatform::Macos => LibTargetPlatform::MacOS,
        TargetPlatform::Android => LibTargetPlatform::Android,
        TargetPlatform::Linux => LibTargetPlatform::Linux,
        TargetPlatform::Windows => LibTargetPlatform::Windows,
        TargetPlatform::Web => panic!("web run should not enter build_and_run"),
        TargetPlatform::Esp32s3 => panic!("esp32 run should not enter build_and_run"),
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
            bail!("Internal error: Android backend requires an Android device")
        }
        _ => Ok(None),
    }
}

fn spawn_device_launch_task(
    device: SelectedDevice,
    needs_launch: bool,
) -> smol::Task<Result<SelectedDevice>> {
    smol::spawn(async move {
        if needs_launch {
            match &device {
                SelectedDevice::AppleSimulator(sim) => sim.launch().await?,
                SelectedDevice::Local(local) => local.launch().await?,
                SelectedDevice::AndroidDevice(dev) => dev.launch().await?,
                SelectedDevice::AndroidEmulator(emu) => emu.launch().await?,
            }
        }
        Ok(device)
    })
}

fn build_options(config: &BuildRunConfig) -> BuildOptions {
    config.sccache_path.as_ref().map_or_else(
        || BuildOptions::new(false),
        |sccache| BuildOptions::new(false).with_sccache(sccache.clone()),
    )
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
            let abi = plan.android_abi.ok_or_else(|| {
                color_eyre::eyre::eyre!("Internal error: missing Android ABI for build")
            })?;
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
) -> Result<Artifact> {
    let package_options = PackageOptions::new(false, true);
    match backend {
        TargetBackend::Apple => package_apple(project, plan.lib_platform, package_options).await,
        TargetBackend::Android => {
            let abi = plan.android_abi.ok_or_else(|| {
                color_eyre::eyre::eyre!("Internal error: missing Android ABI for packaging")
            })?;
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
}

/// Run artifact on device.
async fn run_with_options(
    device: SelectedDevice,
    artifact: Artifact,
    run_options: RunOptions,
) -> Result<Running> {
    let running = match device {
        SelectedDevice::AppleSimulator(sim) => sim.run(artifact, run_options).await?,
        SelectedDevice::Local(local) => local.run(artifact, run_options).await?,
        SelectedDevice::AndroidDevice(dev) => dev.run(artifact, run_options).await?,
        SelectedDevice::AndroidEmulator(emu) => emu.run(artifact, run_options).await?,
    };

    Ok(running)
}

/// A device that can be selected for running.
enum SelectedDevice {
    AppleSimulator(AppleSimulator),
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
            Self::Local(_) | Self::AndroidDevice(_) => false,
            Self::AndroidEmulator(_) => true,
        }
    }
}

async fn check_toolchain_for_backend(
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
                | TargetPlatform::Esp32s3 => {
                    bail!("Internal error: Apple backend is not supported on {platform:?}")
                }
            };
            toolchain_checks::check_apple(sdk).await?;
        }
        TargetBackend::Android => {
            if platform != TargetPlatform::Android {
                bail!("Internal error: Android backend is not supported on {platform:?}");
            }
            toolchain_checks::check_android_run().await?;
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
        TargetBackend::Dew => {
            if platform != TargetPlatform::Esp32s3 {
                bail!("Internal error: dew backend is not supported on {platform:?}");
            }
        }
    }
    Ok(())
}

async fn find_device(
    platform: TargetPlatform,
    backend: TargetBackend,
    device_id: Option<&str>,
) -> Result<SelectedDevice> {
    // For native desktop Rust backends, always use Local device regardless of platform.
    if backend == TargetBackend::Gtk4 || backend == TargetBackend::Hydrolysis {
        return Ok(SelectedDevice::Local(Local));
    }

    match platform {
        TargetPlatform::Ios => {
            let devices = AppleSimulator::scan_ios().await?;

            if let Some(id) = device_id {
                // Find specific device
                for sim in devices {
                    if sim.udid == id || sim.name == id {
                        return Ok(SelectedDevice::AppleSimulator(sim));
                    }
                }
                bail!("Device not found: {id}");
            }

            // Find first booted or first available
            let mut first_available = None;
            for sim in devices {
                if sim.state == "Booted" {
                    return Ok(SelectedDevice::AppleSimulator(sim));
                }
                if first_available.is_none() {
                    first_available = Some(sim);
                }
            }

            first_available
                .map(SelectedDevice::AppleSimulator)
                .ok_or_else(|| color_eyre::eyre::eyre!("No iOS simulators available"))
        }
        TargetPlatform::Macos => {
            // macOS with Apple backend uses the local machine
            Ok(SelectedDevice::Local(Local))
        }
        TargetPlatform::Android => {
            let devices = AndroidDevice::scan().await?;

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
            let avds = AndroidPlatform::list_avds().await?;
            let avd_name = avds.into_iter().next().ok_or_else(|| {
                color_eyre::eyre::eyre!(
                    "No Android devices connected and no emulators available. Create an emulator with Android Studio or `avdmanager`, or connect a device."
                )
            })?;

            Ok(SelectedDevice::AndroidEmulator(
                AndroidEmulator::open(avd_name).await?,
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
            bail!("web platform does not use the device pipeline")
        }
        TargetPlatform::Esp32s3 => {
            bail!("esp32s3 platform does not use the device pipeline")
        }
    }
}

fn device_name(device: &SelectedDevice) -> String {
    match device {
        SelectedDevice::AppleSimulator(sim) => sim.name.clone(),
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
fn handle_device_event(event: Option<DeviceEvent>, platform_name: &str) -> Result<bool> {
    match event {
        Some(DeviceEvent::Started) => {
            shell::status("*", "Application started");
            Ok(false)
        }
        Some(DeviceEvent::Stopped) => {
            shell::status("o", "Application stopped");
            Ok(true)
        }
        Some(DeviceEvent::Stdout { message }) => {
            line!("[stdout] {message}");
            Ok(false)
        }
        Some(DeviceEvent::Stderr { message }) => {
            warn!("[stderr] {message}");
            Ok(false)
        }
        Some(DeviceEvent::Log { level, message }) => {
            shell::device_log(platform_name, level, message);
            Ok(false)
        }
        Some(DeviceEvent::Exited(exit)) => {
            shell::status("o", exit.terminal_message());
            Ok(true)
        }
        Some(DeviceEvent::Crashed(msg)) => {
            // Use panic_report for panic messages, regular error for others
            if msg.starts_with("Panic:") {
                shell::panic_report(&msg);
            } else {
                error!("Application crashed: {msg}");
            }
            bail!("application crashed")
        }
        None => Ok(true),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BackendAvailability, TargetBackend, TargetPlatform, handle_device_event, resolve_backend,
        resolve_default_backend_for_project, resolve_platform,
        validate_desktop_backend_platform_on_host, validate_device_arg,
    };
    use waterui_cli::device::{ApplicationExit, DeviceEvent};

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
        crate::shell::init(false);
        let should_stop = handle_device_event(
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
