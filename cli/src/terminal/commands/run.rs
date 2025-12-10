//! `water run` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};
use futures::StreamExt;

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
    debug::hot_reload::{DEFAULT_PORT, HotReloadServer},
    device::{Artifact, Device, DeviceEvent, RunOptions, Running},
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

    /// Enable hot reload.
    #[arg(long)]
    hot_reload: bool,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,
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
    let running = display_output(async {
        run_on_device(&project, device, needs_launch, args.hot_reload).await
    })
    .await?;

    line!();
    note!("Press Ctrl+C to stop the application");
    line!();

    // Stream device events
    let mut running = std::pin::pin!(running);
    while let Some(event) = running.next().await {
        match event {
            DeviceEvent::Started => {
                shell::status("●", "Application started");
            }
            DeviceEvent::Stopped => {
                shell::status("○", "Application stopped");
                break;
            }
            DeviceEvent::Stdout { message } => {
                line!("[stdout] {message}");
            }
            DeviceEvent::Stderr { message } => {
                warn!("[stderr] {message}");
            }
            DeviceEvent::Log { level, message } => match level {
                tracing::Level::ERROR => error!("{message}"),
                tracing::Level::WARN => warn!("{message}"),
                _ => line!("[{level}] {message}"),
            },
            DeviceEvent::Exited => {
                note!("Application exited");
                break;
            }
            DeviceEvent::Crashed(msg) => {
                error!("Application crashed: {msg}");
                break;
            }
        }
    }

    Ok(())
}

/// Build, launch device in background, and run.
async fn run_on_device(
    project: &Project,
    device: SelectedDevice,
    needs_launch: bool,
    hot_reload: bool,
) -> Result<Running> {
    match device {
        SelectedDevice::AppleSimulator(sim) => {
            let platform = sim.platform();

            // Always spawn task - it will launch if needed, otherwise just return device
            let launch_task = smol::spawn(async move {
                if needs_launch {
                    sim.launch().await?;
                }
                Ok::<_, color_eyre::eyre::Report>(sim)
            });

            // Build and package while device launches in background
            shell::status("▶", "Building...");
            platform.build(project, BuildOptions::new(false)).await?;
            shell::status("▶", "Packaging...");
            let artifact = platform
                .package(project, PackageOptions::new(false, true))
                .await?;

            // Wait for device to be ready
            if needs_launch {
                shell::status("▶", "Waiting for simulator...");
            }
            let sim = launch_task.await?;

            shell::status("▶", "Running...");
            run_with_options(sim, artifact, hot_reload).await
        }
        SelectedDevice::AppleMacos(macos) => {
            let platform = macos.platform();

            // macOS doesn't need launching
            shell::status("▶", "Building...");
            platform.build(project, BuildOptions::new(false)).await?;
            shell::status("▶", "Packaging...");
            let artifact = platform
                .package(project, PackageOptions::new(false, true))
                .await?;

            shell::status("▶", "Running...");
            run_with_options(macos, artifact, hot_reload).await
        }
        SelectedDevice::AndroidDevice(dev) => {
            let platform = dev.platform();

            // Already connected device doesn't need launching
            shell::status("▶", "Building...");
            platform.build(project, BuildOptions::new(false)).await?;
            shell::status("▶", "Packaging...");
            let artifact = platform
                .package(project, PackageOptions::new(false, true))
                .await?;

            shell::status("▶", "Running...");
            run_with_options(dev, artifact, hot_reload).await
        }
        SelectedDevice::AndroidEmulator(emu) => {
            let platform = emu.platform();

            // Always spawn task - it will launch if needed, otherwise just return device
            let launch_task = smol::spawn(async move {
                if needs_launch {
                    emu.launch().await?;
                }
                Ok::<_, color_eyre::eyre::Report>(emu)
            });

            // Build and package while emulator launches in background
            shell::status("▶", "Building...");
            platform.build(project, BuildOptions::new(false)).await?;
            shell::status("▶", "Packaging...");
            let artifact = platform
                .package(project, PackageOptions::new(false, true))
                .await?;

            // Wait for emulator to be ready
            if needs_launch {
                shell::status("▶", "Waiting for emulator...");
            }
            let emu = launch_task.await?;

            shell::status("▶", "Running...");
            run_with_options(emu, artifact, hot_reload).await
        }
    }
}

/// Run artifact on device with hot reload support.
async fn run_with_options<D: Device>(
    device: D,
    artifact: Artifact,
    hot_reload: bool,
) -> Result<Running> {
    let mut run_options = RunOptions::new();

    let server = if hot_reload {
        let server = HotReloadServer::launch(DEFAULT_PORT).await?;
        run_options.insert_env_var("WATERUI_HOT_RELOAD_HOST".to_string(), server.host());
        run_options.insert_env_var(
            "WATERUI_HOT_RELOAD_PORT".to_string(),
            server.port().to_string(),
        );
        Some(server)
    } else {
        None
    };

    let mut running = device.run(artifact, run_options).await?;

    if let Some(server) = server {
        running.retain(server);
    }

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
