//! `water run` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};
use futures::StreamExt;

use crate::shell::{self, display_output};
use crate::{error, header, line, note, success, warn};
use waterui_cli::{
    android::{device::AndroidDevice, platform::AndroidPlatform},
    apple::{
        device::{AppleDevice, AppleSimulator, MacOS},
        platform::ApplePlatform,
    },
    device::DeviceEvent,
    platform::Platform,
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
    success!("Found device: {}", device_name(&device));

    // Step 3: Run on device
    let running = display_output(async {
        match device {
            SelectedDevice::AppleSimulator(sim) => {
                shell::status("▶", "Building and running...");
                project.run(sim, args.hot_reload).await
            }
            SelectedDevice::AppleMacos(macos) => {
                shell::status("▶", "Building and running...");
                project.run(macos, args.hot_reload).await
            }
            SelectedDevice::Android(android) => {
                shell::status("▶", "Building and running...");
                project.run(android, args.hot_reload).await
            }
        }
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

enum SelectedDevice {
    AppleSimulator(AppleSimulator),
    AppleMacos(MacOS),
    Android(AndroidDevice),
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
            let devices = p.scan().await?;

            if let Some(id) = device_id {
                // Find specific device
                for dev in devices {
                    if dev.identifier() == id {
                        return Ok(SelectedDevice::Android(dev));
                    }
                }
                bail!("Device not found: {id}");
            }

            // Use first available
            devices
                .into_iter()
                .next()
                .map(SelectedDevice::Android)
                .ok_or_else(|| color_eyre::eyre::eyre!("No Android devices connected"))
        }
    }
}

fn device_name(device: &SelectedDevice) -> String {
    match device {
        SelectedDevice::AppleSimulator(sim) => sim.name.clone(),
        SelectedDevice::AppleMacos(_) => "Current Machine".to_string(),
        SelectedDevice::Android(dev) => dev.identifier().to_string(),
    }
}

const fn platform_name(platform: TargetPlatform) -> &'static str {
    match platform {
        TargetPlatform::Ios => "iOS Simulator",
        TargetPlatform::Android => "Android",
        TargetPlatform::Macos => "macOS",
    }
}
