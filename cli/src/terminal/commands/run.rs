//! `water run` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};
use futures::{FutureExt, StreamExt};

use crate::shell::{self, display_output};
use crate::{error, header, line, note, success, warn};
use waterui_cli::{
    android::{
        device::{AndroidDevice, AndroidEmulator},
        platform::AndroidPlatform,
    },
    apple::{
        device::{AppleDevice, AppleSimulator, MacOS},
        platform::ApplePlatform,
    },
    build::BuildOptions,
    debug::{HotReloadEvent, HotReloadRunner},
    device::{Artifact, Device, DeviceEvent, LogLevel, RunOptions, Running},
    platform::{PackageOptions, Platform},
    project::Project,
    toolchain::Toolchain,
};

/// Target platform for running.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TargetPlatform {
    /// iOS Simulator.
    Ios,
    /// Android.
    Android,
    /// macOS (current machine).
    Macos,
}

/// Arguments for the run command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Target platform to run on.
    #[arg(short, long, value_enum)]
    platform: TargetPlatform,

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

/// Run the run command.
pub async fn run(args: Args) -> Result<()> {
    let project_path = args
        .path
        .canonicalize()
        .unwrap_or_else(|_| args.path.clone());
    let project = Project::open(&project_path).await?;

    header!(
        "Running {} on {}",
        project.crate_name(),
        platform_name(args.platform)
    );

    // Step 1: Check toolchain
    let spinner = shell::spinner("Checking toolchain...");
    check_toolchain(args.platform).await?;
    if let Some(pb) = spinner {
        pb.finish_and_clear();
    }
    success!("Toolchain ready");

    // Step 2: Find device
    let spinner = shell::spinner("Scanning for devices...");
    let device = find_device(args.platform, args.device.as_deref()).await?;
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
    let (running, hot_reload_runner) = display_output(build_and_run(
        &project,
        device,
        needs_launch,
        hot_reload,
        log_level,
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
    let platform_name = match args.platform {
        TargetPlatform::Android => "Android",
        TargetPlatform::Ios | TargetPlatform::Macos => "Apple",
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

        futures::select! {
            _ = FutureExt::fuse(timeout) => {
                // Timeout - loop back to check hot reload events
                continue;
            }
            dev_event = FutureExt::fuse(device_event) => {
                if handle_device_event(dev_event, platform_name) {
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Build, package, and run on device.
///
/// Handles:
/// - Launching device in background (if needed) while building
/// - Building and packaging via the device's platform
/// - Running with hot reload support
///
/// Returns the running app stream and optionally a hot reload runner.
async fn build_and_run(
    project: &Project,
    device: SelectedDevice,
    needs_launch: bool,
    hot_reload: bool,
    log_level: Option<LogLevel>,
) -> Result<(Running, Option<HotReloadRunner>)> {
    match device {
        SelectedDevice::AppleSimulator(sim) => {
            build_and_run_device(project, sim, needs_launch, hot_reload, log_level).await
        }
        SelectedDevice::AppleMacos(macos) => {
            build_and_run_device(project, macos, needs_launch, hot_reload, log_level).await
        }
        SelectedDevice::AndroidDevice(dev) => {
            build_and_run_device(project, dev, needs_launch, hot_reload, log_level).await
        }
        SelectedDevice::AndroidEmulator(emu) => {
            build_and_run_device(project, emu, needs_launch, hot_reload, log_level).await
        }
    }
}

/// Generic implementation for building and running on any device type.
async fn build_and_run_device<D: Device + 'static>(
    project: &Project,
    device: D,
    needs_launch: bool,
    hot_reload: bool,
    log_level: Option<LogLevel>,
) -> Result<(Running, Option<HotReloadRunner>)>
where
    D::Platform: Platform,
{
    let platform = device.platform();
    let triple = platform.triple();

    // Launch device in background while building (if needed)
    let launch_task = smol::spawn(async move {
        if needs_launch {
            device.launch().await?;
        }
        Ok::<_, color_eyre::eyre::Report>(device)
    });

    // Build and package while device launches in background
    shell::status("▶", "Building...");
    platform
        .build(project, BuildOptions::new(false, hot_reload))
        .await?;
    shell::status("▶", "Packaging...");
    let artifact = platform
        .package(project, PackageOptions::new(false, true))
        .await?;

    // Wait for device to be ready
    if needs_launch {
        shell::status("▶", "Waiting for device...");
    }
    let device = launch_task.await?;

    // Create hot reload runner if enabled
    let runner = if hot_reload {
        shell::status("▶", "Starting hot reload...");
        Some(HotReloadRunner::new(project, triple).await?)
    } else {
        None
    };

    shell::status("▶", "Running...");
    let running = run_with_options(device, artifact, runner.as_ref(), log_level).await?;

    Ok((running, runner))
}

/// Run artifact on device with hot reload support.
async fn run_with_options<D: Device>(
    device: D,
    artifact: Artifact,
    runner: Option<&HotReloadRunner>,
    log_level: Option<LogLevel>,
) -> Result<Running> {
    let mut run_options = RunOptions::new();

    if let Some(level) = log_level {
        run_options.set_log_level(level);
    }

    // Set hot reload env vars if runner is provided
    if let Some(runner) = runner {
        run_options.insert_env_var("WATERUI_HOT_RELOAD_HOST".to_string(), runner.host());
        run_options.insert_env_var(
            "WATERUI_HOT_RELOAD_PORT".to_string(),
            runner.port().to_string(),
        );
    }

    let running = device.run(artifact, run_options).await?;

    Ok(running)
}

/// A device that can be selected for running.
enum SelectedDevice {
    AppleSimulator(AppleSimulator),
    AppleMacos(MacOS),
    AndroidDevice(AndroidDevice),
    AndroidEmulator(AndroidEmulator),
}

impl SelectedDevice {
    /// Check if the device needs to be launched before running.
    fn needs_launch(&self) -> bool {
        match self {
            Self::AppleSimulator(sim) => sim.state != "Booted",
            Self::AppleMacos(_) => false,
            Self::AndroidDevice(_) => false,
            Self::AndroidEmulator(_) => true,
        }
    }
}

async fn check_toolchain(platform: TargetPlatform) -> Result<()> {
    match platform {
        TargetPlatform::Ios | TargetPlatform::Macos => {
            let platform = ApplePlatform::ios_simulator();
            let toolchain = platform.toolchain();
            if let Err(e) = toolchain.check().await {
                bail!("Toolchain check failed: {e}");
            }
        }
        TargetPlatform::Android => {
            let platform = AndroidPlatform::arm64();
            let toolchain = platform.toolchain();
            if let Err(e) = toolchain.check().await {
                bail!("Toolchain check failed: {e}");
            }
        }
    }
    Ok(())
}

async fn find_device(platform: TargetPlatform, device_id: Option<&str>) -> Result<SelectedDevice> {
    match platform {
        TargetPlatform::Ios => {
            let p = ApplePlatform::ios_simulator();
            let devices = p.scan().await?;

            if let Some(id) = device_id {
                // Find specific device
                for dev in devices {
                    if let AppleDevice::Simulator(sim) = dev {
                        if sim.udid == id || sim.name == id {
                            return Ok(SelectedDevice::AppleSimulator(sim));
                        }
                    }
                }
                bail!("Device not found: {id}");
            }

            // Find first booted or first available
            let mut first_available = None;
            for dev in devices {
                if let AppleDevice::Simulator(sim) = dev {
                    if sim.state == "Booted" {
                        return Ok(SelectedDevice::AppleSimulator(sim));
                    }
                    if first_available.is_none() {
                        first_available = Some(sim);
                    }
                }
            }

            first_available
                .map(SelectedDevice::AppleSimulator)
                .ok_or_else(|| color_eyre::eyre::eyre!("No iOS simulators available"))
        }
        TargetPlatform::Macos => {
            // macOS is always the current machine
            Ok(SelectedDevice::AppleMacos(MacOS))
        }
        TargetPlatform::Android => {
            let p = AndroidPlatform::arm64();
            let devices = p.scan().await.unwrap_or_default();

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
    }
}

fn device_name(device: &SelectedDevice) -> String {
    match device {
        SelectedDevice::AppleSimulator(sim) => sim.name.clone(),
        SelectedDevice::AppleMacos(_) => "Current Machine".to_string(),
        SelectedDevice::AndroidDevice(dev) => dev.identifier().to_string(),
        SelectedDevice::AndroidEmulator(emu) => format!("{} (emulator)", emu.avd_name()),
    }
}

const fn platform_name(platform: TargetPlatform) -> &'static str {
    match platform {
        TargetPlatform::Ios => "iOS Simulator",
        TargetPlatform::Android => "Android",
        TargetPlatform::Macos => "macOS",
    }
}

/// Handle a hot reload event, displaying status to the user.
fn handle_hot_reload_event(event: HotReloadEvent) {
    match event {
        HotReloadEvent::ServerStarted { host, port } => {
            shell::status("◉", &format!("Hot reload server on {host}:{port}"));
        }
        HotReloadEvent::FileChanged => {
            shell::status("◌", "File changed, rebuilding...");
        }
        HotReloadEvent::Rebuilding => {
            shell::status("◐", "Building...");
        }
        HotReloadEvent::Built { path } => {
            shell::status("◑", &format!("Built: {}", path.display()));
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
            error!("Application crashed: {msg}");
            true
        }
        None => true,
    }
}
