//! `water devices` command implementation.

use std::collections::HashSet;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};

use crate::shell;
use crate::{header, line};
use smol::future::zip;
use smol::process::Command;
use waterui_cli::{
    android::{
        device::{AndroidDevice, emulator_avd_name_with_adb},
        toolchain::AndroidSdk,
    },
    apple::device::AppleSimulator,
    device::Device,
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
            let ios_devices = scan_ios_devices().await?;
            display_ios_devices(&ios_devices);
        }
        TargetPlatform::Android => {
            let (avds, devices, running_avds) = scan_android_devices().await?;
            display_android_devices(&avds, &devices, &running_avds);
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
            display_ios_devices(&ios_devices?);
            {
                let (avds, devices, running_avds) = android_result?;
                display_android_devices(&avds, &devices, &running_avds);
            }
            display_macos_devices();
        }
    }

    Ok(())
}

/// Scan iOS simulators.
async fn scan_ios_devices() -> Result<Vec<AppleSimulator>> {
    Ok(AppleSimulator::scan_ios().await?)
}

/// Scan Android devices and emulators.
async fn scan_android_devices() -> Result<(Vec<String>, Vec<AndroidDevice>, HashSet<String>)> {
    let emulator_path = AndroidSdk::emulator_path()
        .ok_or_else(|| color_eyre::eyre::eyre!("Android emulator not found"))?;
    let adb_path =
        AndroidSdk::adb_path().ok_or_else(|| color_eyre::eyre::eyre!("Android adb not found"))?;

    // List available AVDs (emulators) and connected devices in parallel
    let avds_future = async {
        Command::new(&emulator_path)
            .arg("-list-avds")
            .output()
            .await
            .map_err(|e| color_eyre::eyre::eyre!("Failed to list AVDs: {e}"))
            .and_then(|output| {
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    bail!("Failed to list AVDs: {}", stderr.trim());
                }
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(stdout
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(String::from)
                    .collect::<Vec<_>>())
            })
    };

    let devices_future = AndroidDevice::scan();

    let (avds, connected_devices) = zip(avds_future, devices_future).await;
    let avds = avds?;
    let connected_devices = connected_devices?;

    // Resolve running emulator AVD names (so we can mark the correct AVDs as "Booted")
    let mut running_avds = HashSet::new();
    for device in &connected_devices {
        let id = device.identifier();
        if !id.starts_with("emulator-") {
            continue;
        }
        let name = emulator_avd_name_with_adb(&adb_path, id).await?;
        running_avds.insert(name);
    }

    Ok((avds, connected_devices, running_avds))
}

/// Display iOS devices.
fn display_ios_devices(devs: &[AppleSimulator]) {
    if !devs.is_empty() {
        header!("iOS Simulators");
    }

    for sim in devs {
        let state_icon = if sim.state == "Booted" { "●" } else { "○" };
        line!("  {} {} ({})", state_icon, sim.name, sim.udid);
    }

    if devs.is_empty() {
        line!("  No iOS simulators available");
    }
}

/// Display Android devices and emulators.
fn display_android_devices(
    avds: &[String],
    connected_devices: &[AndroidDevice],
    running_avds: &HashSet<String>,
) {
    header!("Android");

    // Show emulators
    for avd in avds {
        let is_running = running_avds.contains(avd);
        let state_icon = if is_running { "●" } else { "○" };
        line!("  {} {} (emulator)", state_icon, avd);
    }

    // Show connected physical devices
    for device in connected_devices {
        if !device.identifier().starts_with("emulator-") {
            line!("  ● {} ({})", device.identifier(), device.abi().as_str());
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
