//! `water devices` command implementation.

use std::{collections::HashSet, path::PathBuf};

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};
use serde::Serialize;

use crate::shell;
use crate::{header, line};
use smol::future::zip;
use smol::process::Command;
use waterui_cli::{
    android::{
        AndroidSdk,
        device::{AndroidDevice, emulator_avd_name_with_adb},
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
    if shell::get().is_json() {
        return run_json(args).await;
    }

    match args.platform {
        TargetPlatform::Ios => {
            let ios_devices = scan_ios_devices().await?;
            display_ios_devices(&ios_devices);
        }
        TargetPlatform::Android => {
            let adb_path = resolve_android_adb()?;
            let (avds, devices, running_avds) =
                scan_android_devices(AndroidSdk::emulator_path(), adb_path).await?;
            display_android_devices(&avds, &devices, &running_avds);
        }
        TargetPlatform::Macos => {
            display_macos_devices();
        }
        TargetPlatform::All => {
            // Fast-fail only when adb is unavailable.
            let adb_path = resolve_android_adb()?;
            let spinner = shell::spinner("Scanning devices...");

            // Scan iOS and Android in parallel
            let (ios_devices, android_result) = zip(
                scan_ios_devices(),
                scan_android_devices(AndroidSdk::emulator_path(), adb_path),
            )
            .await;

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

async fn run_json(args: Args) -> Result<()> {
    let output = match args.platform {
        TargetPlatform::Ios => {
            let ios_devices = scan_ios_devices().await?;
            DevicesJsonOutput {
                ty: "devices",
                platform: "ios",
                ios: Some(json_ios_devices(&ios_devices)),
                android: None,
                macos: None,
            }
        }
        TargetPlatform::Android => {
            let adb_path = resolve_android_adb()?;
            let (avds, devices, running_avds) =
                scan_android_devices(AndroidSdk::emulator_path(), adb_path).await?;
            DevicesJsonOutput {
                ty: "devices",
                platform: "android",
                ios: None,
                android: Some(json_android_section(&avds, &devices, &running_avds)),
                macos: None,
            }
        }
        TargetPlatform::Macos => DevicesJsonOutput {
            ty: "devices",
            platform: "macos",
            ios: None,
            android: None,
            macos: Some(vec![JsonMacosDevice {
                id: "local".to_string(),
                name: "Current Machine".to_string(),
            }]),
        },
        TargetPlatform::All => {
            // Fast-fail only when adb is unavailable.
            let adb_path = resolve_android_adb()?;
            let (ios_devices, android_result) = zip(
                scan_ios_devices(),
                scan_android_devices(AndroidSdk::emulator_path(), adb_path),
            )
            .await;
            let ios_devices = ios_devices?;
            let (avds, devices, running_avds) = android_result?;

            DevicesJsonOutput {
                ty: "devices",
                platform: "all",
                ios: Some(json_ios_devices(&ios_devices)),
                android: Some(json_android_section(&avds, &devices, &running_avds)),
                macos: Some(vec![JsonMacosDevice {
                    id: "local".to_string(),
                    name: "Current Machine".to_string(),
                }]),
            }
        }
    };

    shell::json_raw(&serde_json::to_string(&output)?);
    Ok(())
}

/// Scan iOS simulators.
async fn scan_ios_devices() -> Result<Vec<AppleSimulator>> {
    AppleSimulator::scan_ios().await
}

fn resolve_android_adb() -> Result<PathBuf> {
    AndroidSdk::adb_path().ok_or_else(|| color_eyre::eyre::eyre!("Android adb not found"))
}

/// Scan Android devices and emulators.
async fn scan_android_devices(
    emulator_path: Option<PathBuf>,
    adb_path: PathBuf,
) -> Result<(Vec<String>, Vec<AndroidDevice>, HashSet<String>)> {
    // List available AVDs (emulators) and connected devices in parallel
    let avds_future = async move {
        let Some(emulator_path) = emulator_path else {
            return Ok(Vec::new());
        };
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

#[derive(Debug, Serialize)]
struct DevicesJsonOutput {
    #[serde(rename = "type")]
    ty: &'static str,
    platform: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    ios: Option<Vec<JsonIosDevice>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    android: Option<JsonAndroidSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    macos: Option<Vec<JsonMacosDevice>>,
}

#[derive(Debug, Serialize)]
struct JsonIosDevice {
    name: String,
    udid: String,
    state: String,
    available: bool,
}

#[derive(Debug, Serialize)]
struct JsonAndroidSection {
    emulators: Vec<JsonAndroidEmulator>,
    devices: Vec<JsonAndroidDevice>,
}

#[derive(Debug, Serialize)]
struct JsonAndroidEmulator {
    name: String,
    running: bool,
}

#[derive(Debug, Serialize)]
struct JsonAndroidDevice {
    id: String,
    abi: String,
}

#[derive(Debug, Serialize)]
struct JsonMacosDevice {
    id: String,
    name: String,
}

fn json_ios_devices(devices: &[AppleSimulator]) -> Vec<JsonIosDevice> {
    devices
        .iter()
        .map(|sim| JsonIosDevice {
            name: sim.name.clone(),
            udid: sim.udid.clone(),
            state: sim.state.clone(),
            available: sim.is_available,
        })
        .collect()
}

fn json_android_section(
    avds: &[String],
    connected_devices: &[AndroidDevice],
    running_avds: &HashSet<String>,
) -> JsonAndroidSection {
    let emulators = avds
        .iter()
        .map(|avd| JsonAndroidEmulator {
            name: avd.clone(),
            running: running_avds.contains(avd),
        })
        .collect();

    let devices = connected_devices
        .iter()
        .filter(|d| !d.identifier().starts_with("emulator-"))
        .map(|d| JsonAndroidDevice {
            id: d.identifier().to_string(),
            abi: d.abi().as_str().to_string(),
        })
        .collect();

    JsonAndroidSection { emulators, devices }
}

#[cfg(test)]
mod tests {
    use super::{
        JsonAndroidDevice, JsonAndroidEmulator, JsonAndroidSection, JsonIosDevice, JsonMacosDevice,
    };

    #[test]
    fn json_shapes_are_serializable() {
        let ios = JsonIosDevice {
            name: "iPhone".to_string(),
            udid: "UDID".to_string(),
            state: "Booted".to_string(),
            available: true,
        };
        let android = JsonAndroidSection {
            emulators: vec![JsonAndroidEmulator {
                name: "Pixel_9".to_string(),
                running: true,
            }],
            devices: vec![JsonAndroidDevice {
                id: "ABC123".to_string(),
                abi: "arm64-v8a".to_string(),
            }],
        };
        let macos = JsonMacosDevice {
            id: "local".to_string(),
            name: "Current Machine".to_string(),
        };

        let ios_json = serde_json::to_string(&ios).expect("ios JSON");
        let android_json = serde_json::to_string(&android).expect("android JSON");
        let macos_json = serde_json::to_string(&macos).expect("macos JSON");

        assert!(ios_json.contains("\"udid\":\"UDID\""));
        assert!(android_json.contains("\"abi\":\"arm64-v8a\""));
        assert!(macos_json.contains("\"id\":\"local\""));
    }
}
