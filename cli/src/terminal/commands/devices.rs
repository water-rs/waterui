//! `water devices` command implementation.

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::Result;

use crate::shell;
use crate::{header, line, warn};
use smol::future::zip;
use smol::process::Command;
use waterui_cli::{
    android::{device::AndroidDevice, platform::AndroidPlatform, toolchain::AndroidSdk},
    apple::{device::AppleDevice, platform::ApplePlatform},
    platform::Platform,
};

/// Target platform for device listing.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TargetPlatform {
    /// iOS devices and simulators.
    Ios,
    /// Android devices and emulators.
    Android,
    /// macOS (current machine).
    Macos,
    /// All platforms.
    All,
}

/// Arguments for the devices command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Target platform to list devices for.
    #[arg(short, long, value_enum, default_value = "all")]
    platform: TargetPlatform,
}

/// Run the devices command.
pub async fn run(args: Args) -> Result<()> {
    match args.platform {
        TargetPlatform::Ios => {
            let ios_devices = scan_ios_devices().await;
            display_ios_devices(ios_devices);
        }
        TargetPlatform::Android => {
            let android_result = scan_android_devices().await;
            display_android_devices(android_result);
        }
        TargetPlatform::Macos => {
            display_macos_devices();
        }
        TargetPlatform::All => {
            let spinner = shell::spinner("Scanning devices...");

            // Scan iOS and Android in parallel
            let (ios_devices, android_result) =
                zip(scan_ios_devices(), scan_android_devices()).await;

            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }

            // Display results in order
            display_ios_devices(ios_devices);
            display_android_devices(android_result);
            display_macos_devices();
        }
    }

    Ok(())
}

/// Scan iOS simulators.
async fn scan_ios_devices() -> Result<Vec<AppleDevice>, String> {
    let platform = ApplePlatform::ios_simulator();
    platform.scan().await.map_err(|e| e.to_string())
}

/// Scan Android devices and emulators.
async fn scan_android_devices() -> Option<(Vec<String>, Vec<AndroidDevice>)> {
    let emulator_path = AndroidSdk::emulator_path()?;

    // List available AVDs (emulators) and connected devices in parallel
    let avds_future = async {
        Command::new(&emulator_path)
            .arg("-list-avds")
            .output()
            .await
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|output| {
                output
                    .lines()
                    .filter(|line| !line.is_empty())
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };

    let devices_future = async {
        let platform = AndroidPlatform::arm64();
        platform.scan().await.unwrap_or_default()
    };

    let (avds, connected_devices) = zip(avds_future, devices_future).await;
    Some((avds, connected_devices))
}

/// Display iOS devices.
fn display_ios_devices(result: Result<Vec<AppleDevice>, String>) {
    match result {
        Ok(devs) => {
            if !devs.is_empty() {
                header!("iOS Simulators");
            }

            for device in &devs {
                if let AppleDevice::Simulator(sim) = device {
                    let state_icon = if sim.state == "Booted" { "●" } else { "○" };
                    line!("  {} {} ({})", state_icon, sim.name, sim.udid);
                }
            }

            if devs.is_empty() {
                line!("  No iOS simulators available");
            }
        }
        Err(e) => {
            warn!("Failed to scan iOS simulators: {e}");
        }
    }
}

/// Display Android devices and emulators.
fn display_android_devices(result: Option<(Vec<String>, Vec<AndroidDevice>)>) {
    let Some((avds, connected_devices)) = result else {
        // Android SDK not installed, silently skip
        return;
    };

    header!("Android");

    // Show emulators
    for avd in &avds {
        // Check if this emulator is currently running (would show up in connected devices)
        let is_running = connected_devices
            .iter()
            .any(|d| d.identifier().starts_with("emulator-"));
        let state_icon = if is_running { "●" } else { "○" };
        line!("  {} {} (emulator)", state_icon, avd);
    }

    // Show connected physical devices
    for device in &connected_devices {
        if !device.identifier().starts_with("emulator-") {
            line!("  ● {} ({})", device.identifier(), device.abi());
        }
    }

    if avds.is_empty() && connected_devices.is_empty() {
        line!("  No Android devices or emulators available");
    }
}

/// Display macOS device.
fn display_macos_devices() {
    header!("macOS");
    line!("  ● Current Machine");
}
