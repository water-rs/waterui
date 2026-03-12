//! `water run` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};
use futures::StreamExt;

#[cfg(target_os = "macos")]
use time::OffsetDateTime;

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
    started_at: OffsetDateTime,
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
            started_at: OffsetDateTime::now_utc(),
            device_identifier,
            bundle_id: project.bundle_identifier().to_string(),
            process_name,
        }))
    }

    fn refresh_start(&mut self) {
        self.started_at = OffsetDateTime::now_utc();
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

    /// Include all native platform logs (NSLog, print, etc.), not just WaterUI logs.
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
        TargetPlatform::Ios => TargetBackend::Apple,
        TargetPlatform::Macos => TargetBackend::Apple,
        TargetPlatform::Android => TargetBackend::Android,
        TargetPlatform::Linux => TargetBackend::Gtk4,
        TargetPlatform::Windows => TargetBackend::Hydrolysis,
        TargetPlatform::Web => TargetBackend::Hydrolysis,
    };

    let backend = backend_override.unwrap_or(default_backend);

    // Validate backend supports platform
    let supported = match (platform, backend) {
        // Apple backend: iOS, macOS
        (TargetPlatform::Ios, TargetBackend::Apple) => true,
        (TargetPlatform::Macos, TargetBackend::Apple) => true,
        // Android backend: Android
        (TargetPlatform::Android, TargetBackend::Android) => true,
        // GTK4 backend: Linux
        (TargetPlatform::Linux, TargetBackend::Gtk4) => true,
        // Hydrolysis backend: macOS, Linux, Windows
        (TargetPlatform::Macos, TargetBackend::Hydrolysis) => true,
        (TargetPlatform::Linux, TargetBackend::Hydrolysis) => true,
        (TargetPlatform::Windows, TargetBackend::Hydrolysis) => true,
        (TargetPlatform::Web, TargetBackend::Hydrolysis) => true,
        // All other combinations are invalid
        _ => false,
    };

    if !supported {
        bail!(
            "Backend {:?} does not support platform {:?}.\n\
             Valid combinations:\n  \
             - iOS: apple\n  \
             - macOS: apple, hydrolysis\n  \
             - Android: android\n  \
             - Linux: gtk4, hydrolysis\n  \
             - Windows: hydrolysis\n  \
             - Web: hydrolysis",
            backend,
            platform
        );
    }

    Ok(backend)
}

fn default_backend_priority(platform: TargetPlatform) -> &'static [TargetBackend] {
    match platform {
        TargetPlatform::Ios => &[TargetBackend::Apple],
        TargetPlatform::Android => &[TargetBackend::Android],
        TargetPlatform::Macos => &[TargetBackend::Apple, TargetBackend::Hydrolysis],
        TargetPlatform::Linux => &[TargetBackend::Gtk4, TargetBackend::Hydrolysis],
        TargetPlatform::Windows => &[TargetBackend::Hydrolysis],
        TargetPlatform::Web => &[TargetBackend::Hydrolysis],
    }
}

const fn has_configured_backend(
    backend: TargetBackend,
    has_apple: bool,
    has_android: bool,
    has_gtk4: bool,
    has_hydrolysis: bool,
) -> bool {
    match backend {
        TargetBackend::Apple => has_apple,
        TargetBackend::Android => has_android,
        TargetBackend::Gtk4 => has_gtk4,
        TargetBackend::Hydrolysis => has_hydrolysis,
    }
}

fn resolve_default_backend_for_project(
    platform: TargetPlatform,
    project_is_playground: bool,
    has_apple: bool,
    has_android: bool,
    has_gtk4: bool,
    has_hydrolysis: bool,
) -> TargetBackend {
    let backends = default_backend_priority(platform);
    if project_is_playground {
        return backends[0];
    }

    for backend in backends {
        if has_configured_backend(*backend, has_apple, has_android, has_gtk4, has_hydrolysis) {
            return *backend;
        }
    }

    backends[0]
}

fn resolve_platform(platform_override: Option<TargetPlatform>) -> Result<TargetPlatform> {
    if let Some(platform) = platform_override {
        return Ok(platform);
    }

    #[cfg(target_os = "macos")]
    {
        return Ok(TargetPlatform::Macos);
    }
    #[cfg(target_os = "linux")]
    {
        return Ok(TargetPlatform::Linux);
    }
    #[cfg(target_os = "windows")]
    {
        return Ok(TargetPlatform::Windows);
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!(
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
    let project_path = crate::project_path::canonicalize(&args.path)?;
    let mut project = Project::open(&project_path).await?;
    let platform = resolve_platform(args.platform)?;

    // Resolve the backend to use
    let backend = match args.backend {
        Some(backend_override) => resolve_backend(platform, Some(backend_override))?,
        None => {
            let selected = resolve_default_backend_for_project(
                platform,
                project.is_playground(),
                project.apple_backend().is_some(),
                project.android_backend().is_some(),
                project.gtk4_backend().is_some(),
                project.hydrolysis_backend().is_some(),
            );
            resolve_backend(platform, Some(selected))?
        }
    };
    validate_desktop_backend_platform_on_host(platform, backend)?;
    validate_device_arg(platform, backend, args.device.as_deref())?;
    validate_web_log_args(platform, args.logs, args.native_logs)?;

    header!(
        "Running {} on {} ({})",
        project.crate_name(),
        platform_name(platform),
        backend_name(backend)
    );

    // Step 1: Check toolchain
    let spinner = shell::spinner("Checking toolchain...");
    check_toolchain_for_backend(platform, backend).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Toolchain ready");

    // Validate backend presence for app mode and lazily initialize generated backends in playground mode.
    if !project.is_playground() {
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
    }

    // Playground mode keeps generated backends in `.water`; regenerate managed backend glue when needed.
    match backend {
        TargetBackend::Gtk4 if project.is_playground() => {
            let needs_reinit = project.gtk4_backend().is_none()
                || !project
                    .backend_path::<Gtk4Backend>()
                    .join("Cargo.toml")
                    .exists();
            if needs_reinit {
                let spinner = shell::spinner("Initializing GTK4 backend...");
                reinit_backend::<Gtk4Backend>(&project).await?;
                project = Project::open(&project_path).await?;
                if let Some(pb) = spinner {
                    pb.finish_and_clear();
                }
                success!("GTK4 backend initialized");
            }
        }
        TargetBackend::Hydrolysis if project.is_playground() => {
            let needs_reinit = project.hydrolysis_backend().is_none()
                || HydrolysisBackend::requires_regeneration(&project)?;
            if needs_reinit {
                let spinner = shell::spinner("Initializing hydrolysis backend...");
                reinit_backend::<HydrolysisBackend>(&project).await?;
                project = Project::open(&project_path).await?;
                if let Some(pb) = spinner {
                    pb.finish_and_clear();
                }
                success!("Hydrolysis backend initialized");
            }
        }
        _ => {}
    }

    if args.platform == TargetPlatform::Web {
        let spinner = shell::spinner("Building Hydrolysis web app...");
        let site_root = prepare_hydrolysis_web_dev_site(&project).await?;
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

    // Step 2: Find device
    let spinner = shell::spinner("Scanning for devices...");
    let device = find_device(platform, backend, args.device.as_deref()).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }

    // Check if device needs launching
    let needs_launch = device.needs_launch();
    if needs_launch {
        note!("Will launch: {}", device_name(&device));
    } else {
        success!("Found device: {}", device_name(&device));
    }

    // Step 3: Build, package, launch device, and run
    // Launch happens in background while building for efficiency
    let log_level = args.logs.map(LogLevel::from);
    let native_logs = args.native_logs;

    // Detect sccache for compilation caching unless explicitly disabled.
    let sccache_path = if sccache_allowed() {
        let sccache = Sccache;
        match sccache.path().await {
            Ok(path) => Some(path),
            Err(_) => {
                warn!(
                    "sccache not found. Build efficiency may be reduced. Install with: {}",
                    sccache_install_hint()
                );
                None
            }
        }
    } else {
        note!("Skipping sccache (explicit wrapper or WATERUI_DISABLE_SCCACHE is set)");
        None
    };

    #[cfg(target_os = "macos")]
    let mut crash_ctx = match CrashReportContext::try_new(&project, platform, backend) {
        Ok(ctx) => ctx,
        Err(e) => {
            warn!("Crash report augmentation disabled: {e}");
            None
        }
    };

    let running = display_output(build_and_run(
        &project,
        platform,
        backend,
        device,
        needs_launch,
        log_level,
        native_logs,
        sccache_path,
    ))
    .await?;

    line!();
    note!("Press Ctrl+C to stop the application");
    line!();

    // Stream device events
    let mut running = std::pin::pin!(running);
    let backend_log_name = match backend {
        TargetBackend::Apple => "Apple",
        TargetBackend::Android => "Android",
        TargetBackend::Gtk4 => "GTK4",
        TargetBackend::Hydrolysis => "Hydrolysis",
    };

    loop {
        let event = running.next().await;

        #[cfg(target_os = "macos")]
        let mut event = event;

        #[cfg(target_os = "macos")]
        if let Some(DeviceEvent::Started) = event.as_ref() {
            if let Some(ref mut ctx) = crash_ctx {
                ctx.refresh_start();
            }
        }

        #[cfg(target_os = "macos")]
        if let Some(ref ctx) = crash_ctx {
            event = augment_event_with_crash_report(event, ctx).await;
        }

        if handle_device_event(event, backend_log_name) {
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
    match event {
        Some(DeviceEvent::Exited) => {
            if let Some(report) = find_latest_ips_report(ctx).await {
                return Some(DeviceEvent::Crashed(report.to_string()));
            }
            Some(DeviceEvent::Exited)
        }
        Some(DeviceEvent::Crashed(mut msg)) => {
            if !msg.contains("Crash report:") {
                if let Some(report) = find_latest_ips_report(ctx).await {
                    msg.push_str(&format!(
                        "\n\nCrash report: {}",
                        report.log_path().display()
                    ));
                }
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
    log_level: Option<LogLevel>,
    native_logs: bool,
    sccache_path: Option<PathBuf>,
) -> Result<Running> {
    let lib_platform = match cli_platform {
        TargetPlatform::Ios => LibTargetPlatform::IOSSimulator,
        TargetPlatform::Macos => LibTargetPlatform::MacOS,
        TargetPlatform::Android => LibTargetPlatform::Android,
        TargetPlatform::Linux => LibTargetPlatform::Linux,
        TargetPlatform::Windows => LibTargetPlatform::Windows,
        TargetPlatform::Web => panic!("web run should not enter build_and_run"),
    };

    let android_abi = match (backend, &device) {
        (TargetBackend::Android, SelectedDevice::AndroidDevice(dev)) => Some(dev.abi()),
        (TargetBackend::Android, SelectedDevice::AndroidEmulator(emu)) => Some(emu.expected_abi()),
        (TargetBackend::Android, _) => {
            bail!("Internal error: Android backend requires an Android device")
        }
        _ => None,
    };

    // Launch device in background while building (if needed)
    let launch_task = smol::spawn(async move {
        if needs_launch {
            match &device {
                SelectedDevice::AppleSimulator(sim) => sim.launch().await?,
                SelectedDevice::Local(local) => local.launch().await?,
                SelectedDevice::AndroidDevice(dev) => dev.launch().await?,
                SelectedDevice::AndroidEmulator(emu) => emu.launch().await?,
            }
        }
        Ok::<_, color_eyre::eyre::Report>(device)
    });

    // Build and package while device launches in background
    shell::status(">", "Building...");
    let mut build_options = BuildOptions::new(false);
    if let Some(ref sccache) = sccache_path {
        build_options = build_options.with_sccache(sccache.clone());
    }

    // Build based on backend, not platform
    match backend {
        TargetBackend::Apple => {
            build_rust_lib(project, lib_platform, build_options).await?;
        }
        TargetBackend::Android => {
            let abi = android_abi.ok_or_else(|| {
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
            build_hydrolysis(project, lib_platform, build_options).await?;
        }
    }

    shell::status(">", "Packaging...");
    let package_options = PackageOptions::new(false, true);

    // Package based on backend, not platform
    let artifact = match backend {
        TargetBackend::Apple => package_apple(project, lib_platform, package_options).await?,
        TargetBackend::Android => {
            let abi = android_abi.ok_or_else(|| {
                color_eyre::eyre::eyre!("Internal error: missing Android ABI for packaging")
            })?;
            AndroidPlatform::package_with_abis(project, package_options, &[abi]).await?
        }
        TargetBackend::Gtk4 => package_gtk4(project, package_options).await?,
        TargetBackend::Hydrolysis => {
            package_hydrolysis(project, lib_platform, package_options).await?
        }
    };

    // Wait for device to be ready
    if needs_launch {
        shell::status(">", "Waiting for device...");
    }
    let device = launch_task.await?;

    shell::status(">", "Running...");
    let running = run_with_options(device, artifact, log_level, native_logs).await?;

    Ok(running)
}

/// Run artifact on device.
async fn run_with_options(
    device: SelectedDevice,
    artifact: Artifact,
    log_level: Option<LogLevel>,
    native_logs: bool,
) -> Result<Running> {
    let mut run_options = RunOptions::new();

    if let Some(level) = log_level {
        run_options.set_log_level(level);
    }
    run_options.set_native_logs(native_logs);

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
                | TargetPlatform::Web => {
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

fn validate_web_log_args(
    platform: TargetPlatform,
    logs: Option<CliLogLevel>,
    native_logs: bool,
) -> Result<()> {
    if platform != TargetPlatform::Web {
        return Ok(());
    }
    if logs.is_some() {
        bail!("--logs is not supported with the web platform");
    }
    if native_logs {
        bail!("--native-logs is not supported with the web platform");
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

/// Handle a device event.
///
/// Returns `true` if the event loop should break.
fn handle_device_event(event: Option<DeviceEvent>, platform_name: &str) -> bool {
    match event {
        Some(DeviceEvent::Started) => {
            shell::status("*", "Application started");
            false
        }
        Some(DeviceEvent::Stopped) => {
            shell::status("o", "Application stopped");
            true
        }
        Some(DeviceEvent::Stdout { message }) => {
            line!("[stdout] {message}");
            false
        }
        Some(DeviceEvent::Stderr { message }) => {
            warn!("[stderr] {message}");
            false
        }
        Some(DeviceEvent::Log { level, message }) => {
            shell::device_log(platform_name, level, message);
            false
        }
        Some(DeviceEvent::Exited) => {
            note!("Application exited");
            true
        }
        Some(DeviceEvent::Crashed(msg)) => {
            // Use panic_report for panic messages, regular error for others
            if msg.starts_with("Panic:") {
                shell::panic_report(&msg);
            } else {
                error!("Application crashed: {msg}");
            }
            true
        }
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TargetBackend, TargetPlatform, resolve_backend, resolve_default_backend_for_project,
        resolve_platform, validate_desktop_backend_platform_on_host, validate_device_arg,
    };

    #[test]
    fn rejects_device_with_desktop_backend() {
        let err = validate_device_arg(TargetPlatform::Linux, TargetBackend::Gtk4, Some("foo"))
            .expect_err("gtk4 with --device should fail");
        assert!(err.to_string().contains("--device is not supported"));
        let err = validate_device_arg(TargetPlatform::Linux, TargetBackend::Hydrolysis, Some("foo"))
            .expect_err("hydrolysis with --device should fail");
        assert!(err.to_string().contains("--device is not supported"));
    }

    #[test]
    fn accepts_device_with_non_gtk4_backend() {
        assert!(validate_device_arg(TargetPlatform::Ios, TargetBackend::Apple, Some("sim-1")).is_ok());
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
                false,
                false,
                false,
                true
            ),
            TargetBackend::Hydrolysis
        );
        assert_eq!(
            resolve_default_backend_for_project(
                TargetPlatform::Macos,
                false,
                false,
                false,
                false,
                true
            ),
            TargetBackend::Hydrolysis
        );
        assert_eq!(
            resolve_default_backend_for_project(
                TargetPlatform::Linux,
                false,
                false,
                false,
                false,
                false
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
                false,
                false,
                false,
                true
            ),
            TargetBackend::Apple
        );
        assert_eq!(
            resolve_default_backend_for_project(
                TargetPlatform::Linux,
                true,
                false,
                false,
                false,
                true
            ),
            TargetBackend::Gtk4
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_platform_defaults_to_host_on_macos() {
        assert_eq!(
            resolve_platform(None).expect("default platform"),
            TargetPlatform::Macos
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolve_platform_defaults_to_host_on_linux() {
        assert_eq!(
            resolve_platform(None).expect("default platform"),
            TargetPlatform::Linux
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn resolve_platform_defaults_to_host_on_windows() {
        assert_eq!(
            resolve_platform(None).expect("default platform"),
            TargetPlatform::Windows
        );
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
