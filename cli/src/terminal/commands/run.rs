//! `water run` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};
use futures::{FutureExt, StreamExt};

#[cfg(target_os = "macos")]
use time::OffsetDateTime;

use crate::shell::{self, display_output};
use crate::{error, header, line, note, success, warn};
use waterui_cli::{
    android::{
        device::{AndroidDevice, AndroidEmulator},
        platform::{build_android, package_android, AndroidPlatform},
        toolchain::{AndroidNdk, AndroidSdk},
    },
    apple::{
        device::AppleSimulator,
        platform::{build_rust_lib, package_apple},
        toolchain::{AppleSdk, Xcode},
    },
    backend::reinit_backend,
    build::BuildOptions,
    debug::{HotReloadEvent, HotReloadRunner},
    device::{Artifact, Device, DeviceEvent, Local, LogLevel, RunOptions, Running},
    gtk4::{
        backend::Gtk4Backend,
        platform::{build_gtk4, package_gtk4},
        toolchain::Gtk4Toolchain,
    },
    platform::{PackageOptions, TargetPlatform as LibTargetPlatform},
    project::Project,
    toolchain::{Toolchain, cmake::Cmake},
};

#[cfg(target_os = "macos")]
use waterui_cli::debug;
#[cfg(target_os = "macos")]
use waterui_cli::project::PackageType;

#[cfg(target_os = "macos")]
struct CrashReportContext {
    started_at: OffsetDateTime,
    bundle_id: String,
    process_name: String,
}

#[cfg(target_os = "macos")]
impl CrashReportContext {
    fn new(project: &Project, platform: TargetPlatform, backend: TargetBackend) -> Option<Self> {
        if platform != TargetPlatform::Macos || backend != TargetBackend::Apple {
            return None;
        }

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

        Some(Self {
            started_at: OffsetDateTime::now_utc(),
            bundle_id: project.bundle_identifier().to_string(),
            process_name,
        })
    }

    fn refresh_start(&mut self) {
        self.started_at = OffsetDateTime::now_utc();
    }
}

#[cfg(target_os = "macos")]
async fn find_latest_ips_report(
    ctx: &CrashReportContext,
) -> Option<debug::CrashReport> {
    let device_identifier = whoami::fallible::hostname().unwrap_or_else(|_| "unknown".into());
    debug::find_macos_ips_crash_report_since(
        "macOS",
        &device_identifier,
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
}

/// Target backend for running (how the app is built and rendered).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TargetBackend {
    /// Apple backend (UIKit/AppKit).
    Apple,
    /// Android backend (Android Views).
    Android,
    /// GTK4 backend (Linux/macOS/Windows).
    Gtk4,
}

/// Arguments for the run command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Target platform to run on.
    #[arg(short, long, value_enum)]
    platform: TargetPlatform,

    /// Backend to use (overrides default for platform).
    /// E.g., `--platform macos --backend gtk4` runs macOS app with GTK4 rendering.
    #[arg(short, long, value_enum)]
    backend: Option<TargetBackend>,

    /// Device identifier (if not specified, uses first available device).
    #[arg(short, long)]
    device: Option<String>,

    /// Disable hot reload (hot reload is enabled by default).
    #[arg(long)]
    no_hot_reload: bool,

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
    };

    let backend = backend_override.unwrap_or(default_backend);

    // Validate backend supports platform
    let supported = match (platform, backend) {
        // Apple backend: iOS, macOS
        (TargetPlatform::Ios, TargetBackend::Apple) => true,
        (TargetPlatform::Macos, TargetBackend::Apple) => true,
        // Android backend: Android
        (TargetPlatform::Android, TargetBackend::Android) => true,
        // GTK4 backend: macOS, Linux
        (TargetPlatform::Macos, TargetBackend::Gtk4) => true,
        (TargetPlatform::Linux, TargetBackend::Gtk4) => true,
        // All other combinations are invalid
        _ => false,
    };

    if !supported {
        bail!(
            "Backend {:?} does not support platform {:?}.\n\
             Valid combinations:\n  \
             - iOS: apple\n  \
             - macOS: apple, gtk4\n  \
             - Android: android\n  \
             - Linux: gtk4",
            backend,
            platform
        );
    }

    Ok(backend)
}

/// Run the run command.
pub async fn run(args: Args) -> Result<()> {
    let project_path = args
        .path
        .canonicalize()
        .unwrap_or_else(|_| args.path.clone());
    let mut project = Project::open(&project_path).await?;

    // Resolve the backend to use
    let backend = resolve_backend(args.platform, args.backend)?;

    header!(
        "Running {} on {} ({})",
        project.crate_name(),
        platform_name(args.platform),
        backend_name(backend)
    );

    // Step 1: Check toolchain
    let spinner = shell::spinner("Checking toolchain...");
    check_toolchain_for_backend(backend).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Toolchain ready");

    // Initialize GTK4 backend if needed (lazy initialization)
    if backend == TargetBackend::Gtk4 && project.gtk4_backend().is_none() {
        let spinner = shell::spinner("Initializing GTK4 backend...");
        reinit_backend::<Gtk4Backend>(&project).await?;
        // Reload project to pick up the new backend
        project = Project::open(&project_path).await?;
        if let Some(pb) = spinner {
            pb.finish_and_clear();
        }
        success!("GTK4 backend initialized");
    }

    // Step 2: Find device
    let spinner = shell::spinner("Scanning for devices...");
    let device = find_device(args.platform, backend, args.device.as_deref()).await?;
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
    let hot_reload = !args.no_hot_reload;
    let native_logs = args.native_logs;

    #[cfg(target_os = "macos")]
    let mut crash_ctx = CrashReportContext::new(&project, args.platform, backend);

    let (running, hot_reload_runner) = display_output(build_and_run(
        &project,
        args.platform,
        backend,
        device,
        needs_launch,
        hot_reload,
        log_level,
        native_logs,
    ))
    .await?;

    line!();
    if hot_reload_runner.is_some() {
        note!("Hot reload enabled - editing source files will update the app");
    }
    note!("Press Ctrl+C to stop the application");
    line!();

    // Stream device events and hot reload events
    let mut running = std::pin::pin!(running);
    let backend_log_name = match backend {
        TargetBackend::Apple => "Apple",
        TargetBackend::Android => "Android",
        TargetBackend::Gtk4 => "GTK4",
    };

    // Get hot reload event receiver if available
    let hot_reload_rx = hot_reload_runner.as_ref().map(|r| r.events().clone());

    loop {
        // Drain all pending hot reload events first (non-blocking)
        if let Some(ref rx) = hot_reload_rx {
            while let Ok(event) = rx.try_recv() {
                handle_hot_reload_event(event);
            }
        }

        // Wait for next event with a short timeout so we can check hot reload events periodically
        let timeout = smol::Timer::after(std::time::Duration::from_millis(100));
        let device_event = running.next();
        let mut pending_event: Option<Option<DeviceEvent>> = None;
        futures::select! {
            _ = FutureExt::fuse(timeout) => {
                // Timeout - loop back to check hot reload events
            }
            dev_event = FutureExt::fuse(device_event) => {
                pending_event = Some(dev_event);
            }
        }

        if let Some(mut event) = pending_event {
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
                    msg.push_str(&format!("\n\nCrash report: {}", report.log_path().display()));
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
    hot_reload: bool,
    log_level: Option<LogLevel>,
    native_logs: bool,
) -> Result<(Running, Option<HotReloadRunner>)> {
    // Get the library target platform for this CLI platform
    let lib_platform = match cli_platform {
        TargetPlatform::Ios => LibTargetPlatform::IOSSimulator,
        TargetPlatform::Macos => LibTargetPlatform::MacOS,
        TargetPlatform::Android => LibTargetPlatform::Android,
        TargetPlatform::Linux => LibTargetPlatform::Linux,
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
    shell::status("▶", "Building...");
    let build_options = BuildOptions::new(false, hot_reload);

    // Build based on backend, not platform
    match backend {
        TargetBackend::Apple => {
            build_rust_lib(project, lib_platform, build_options).await?;
        }
        TargetBackend::Android => {
            build_android(project, lib_platform, build_options).await?;
        }
        TargetBackend::Gtk4 => {
            build_gtk4(project, build_options).await?;
        }
    }

    shell::status("▶", "Packaging...");
    let package_options = PackageOptions::new(false, true);

    // Package based on backend, not platform
    let artifact = match backend {
        TargetBackend::Apple => package_apple(project, lib_platform, package_options).await?,
        TargetBackend::Android => package_android(project, lib_platform, package_options).await?,
        TargetBackend::Gtk4 => package_gtk4(project, package_options).await?,
    };

    // Wait for device to be ready
    if needs_launch {
        shell::status("▶", "Waiting for device...");
    }
    let device = launch_task.await?;

    // Create hot reload runner if enabled
    let triple = lib_platform.triple();
    let runner = if hot_reload {
        shell::status("▶", "Starting hot reload...");
        Some(HotReloadRunner::new(project, triple).await?)
    } else {
        None
    };

    shell::status("▶", "Running...");
    let running = run_with_options(device, artifact, runner.as_ref(), log_level, native_logs).await?;

    Ok((running, runner))
}

/// Run artifact on device with hot reload support.
async fn run_with_options(
    device: SelectedDevice,
    artifact: Artifact,
    runner: Option<&HotReloadRunner>,
    log_level: Option<LogLevel>,
    native_logs: bool,
) -> Result<Running> {
    let mut run_options = RunOptions::new();

    if let Some(level) = log_level {
        run_options.set_log_level(level);
    }
    run_options.set_native_logs(native_logs);

    // Set hot reload env vars if runner is provided
    if let Some(runner) = runner {
        run_options.insert_env_var("WATERUI_HOT_RELOAD_HOST".to_string(), runner.host());
        run_options.insert_env_var(
            "WATERUI_HOT_RELOAD_PORT".to_string(),
            runner.port().to_string(),
        );
    }

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
    /// Local machine - used for macOS (Apple backend) and any platform with GTK4 backend
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

async fn check_toolchain_for_backend(backend: TargetBackend) -> Result<()> {
    match backend {
        TargetBackend::Apple => {
            let xcode = Xcode;
            if let Err(e) = xcode.check().await {
                bail!("Toolchain check failed: {e}");
            }
            let sdk = AppleSdk::Ios;
            if let Err(e) = sdk.check().await {
                bail!("Toolchain check failed: {e}");
            }
        }
        TargetBackend::Android => {
            let sdk = AndroidSdk;
            if let Err(e) = sdk.check().await {
                bail!("Toolchain check failed: {e}");
            }
            let ndk = AndroidNdk;
            if let Err(e) = ndk.check().await {
                bail!("Toolchain check failed: {e}");
            }
            let cmake = Cmake {};
            if let Err(e) = cmake.check().await {
                bail!("Toolchain check failed: {e}");
            }
        }
        TargetBackend::Gtk4 => {
            let toolchain = Gtk4Toolchain;
            if let Err(e) = toolchain.check().await {
                bail!("Toolchain check failed: {e}");
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
    // For GTK4 backend, always use Local device regardless of platform
    if backend == TargetBackend::Gtk4 {
        return Ok(SelectedDevice::Local(Local));
    }

    match platform {
        TargetPlatform::Ios => {
            let devices = AppleSimulator::scan().await?;

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
            let devices = AndroidDevice::scan().await.unwrap_or_default();

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
                    "No Android devices connected and no emulators available. \
                     Create an emulator in Android Studio or connect a device."
                )
            })?;

            Ok(SelectedDevice::AndroidEmulator(AndroidEmulator::new(
                avd_name,
            )))
        }
        TargetPlatform::Linux => {
            // Linux runs on the local machine
            Ok(SelectedDevice::Local(Local))
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
    }
}

const fn backend_name(backend: TargetBackend) -> &'static str {
    match backend {
        TargetBackend::Apple => "Apple",
        TargetBackend::Android => "Android",
        TargetBackend::Gtk4 => "GTK4",
    }
}

/// Handle a hot reload event, displaying status to the user.
fn handle_hot_reload_event(event: HotReloadEvent) {
    match event {
        HotReloadEvent::ServerStarted { host, port } => {
            shell::status("◉", format!("Hot reload server on {host}:{port}"));
        }
        HotReloadEvent::FileChanged => {
            shell::status("◌", "File changed, rebuilding...");
        }
        HotReloadEvent::Rebuilding => {
            shell::status("◐", "Building...");
        }
        HotReloadEvent::Built { path } => {
            shell::status("◑", format!("Built: {}", path.display()));
        }
        HotReloadEvent::BuildFailed { error } => {
            error!("Build failed: {error}");
        }
        HotReloadEvent::Broadcast => {
            success!("Hot reload: updated");
        }
    }
}

/// Handle a device event.
///
/// Returns `true` if the event loop should break.
fn handle_device_event(event: Option<DeviceEvent>, platform_name: &str) -> bool {
    match event {
        Some(DeviceEvent::Started) => {
            shell::status("●", "Application started");
            false
        }
        Some(DeviceEvent::Stopped) => {
            shell::status("○", "Application stopped");
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
