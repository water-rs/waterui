//! `water devices` command implementation.

use std::{collections::HashSet, path::PathBuf};

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};
use serde::Serialize;

use crate::shell::Shell;
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
    esp32::platform::{SerialPortSummary, scan_serial_ports},
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
    /// ESP32 boards on serial ports.
    Esp32,
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
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    if shell.is_json() {
        return run_json(shell, args).await;
    }

    match args.platform {
        TargetPlatform::Ios => {
            let ios_devices = scan_ios_devices().await?;
            display_ios_devices(shell, &ios_devices);
        }
        TargetPlatform::Android => {
            let adb_path = resolve_android_adb()?;
            let (avds, devices, running_avds) =
                scan_android_devices(AndroidSdk::emulator_path(), adb_path).await?;
            display_android_devices(shell, &avds, &devices, &running_avds);
        }
        TargetPlatform::Macos => {
            display_macos_devices(shell);
        }
        TargetPlatform::Esp32 => {
            let ports = scan_serial_ports().await?;
            display_esp32_devices(shell, &ports);
        }
        TargetPlatform::All => {
            // Fast-fail only when adb is unavailable.
            let adb_path = resolve_android_adb()?;
            let spinner = shell.spinner("Scanning devices...");

            // Scan iOS, Android, and serial ports in parallel
            let ((ios_devices, android_result), serial_ports) = zip(
                zip(
                    scan_ios_devices(),
                    scan_android_devices(AndroidSdk::emulator_path(), adb_path),
                ),
                scan_serial_ports(),
            )
            .await;

            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }

            // Display results in order
            display_ios_devices(shell, &ios_devices?);
            {
                let (avds, devices, running_avds) = android_result?;
                display_android_devices(shell, &avds, &devices, &running_avds);
            }
            display_macos_devices(shell);
            display_esp32_devices(shell, &serial_ports?);
        }
    }

    Ok(())
}

async fn run_json(shell: &Shell, args: Args) -> Result<()> {
    let output = match args.platform {
        TargetPlatform::Ios => {
            let ios_devices = scan_ios_devices().await?;
            DevicesJsonOutput {
                ty: "devices",
                platform: "ios",
                ios: Some(json_ios_devices(&ios_devices)),
                android: None,
                macos: None,
                esp32: None,
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
                esp32: None,
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
            esp32: None,
        },
        TargetPlatform::Esp32 => {
            let ports = scan_serial_ports().await?;
            DevicesJsonOutput {
                ty: "devices",
                platform: "esp32",
                ios: None,
                android: None,
                macos: None,
                esp32: Some(json_esp32_devices(&ports)),
            }
        }
        TargetPlatform::All => {
            // Fast-fail only when adb is unavailable.
            let adb_path = resolve_android_adb()?;
            let ((ios_devices, android_result), serial_ports) = zip(
                zip(
                    scan_ios_devices(),
                    scan_android_devices(AndroidSdk::emulator_path(), adb_path),
                ),
                scan_serial_ports(),
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
                esp32: Some(json_esp32_devices(&serial_ports?)),
            }
        }
    };

    let _ = shell.json_raw(&serde_json::to_string(&output)?);
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
fn display_ios_devices(shell: &Shell, devs: &[AppleSimulator]) {
    if !devs.is_empty() {
        header!(shell, "iOS Simulators");
    }

    for sim in devs {
        let state_icon = if sim.state == "Booted" { "●" } else { "○" };
        line!(shell, "  {} {} ({})", state_icon, sim.name, sim.udid);
    }

    if devs.is_empty() {
        line!(shell, "  No iOS simulators available");
    }
}

/// Display Android devices and emulators.
fn display_android_devices(
    shell: &Shell,
    avds: &[String],
    connected_devices: &[AndroidDevice],
    running_avds: &HashSet<String>,
) {
    header!(shell, "Android");

    // Show emulators
    for avd in avds {
        let is_running = running_avds.contains(avd);
        let state_icon = if is_running { "●" } else { "○" };
        line!(shell, "  {} {} (emulator)", state_icon, avd);
    }

    // Show connected physical devices
    for device in connected_devices {
        if !device.identifier().starts_with("emulator-") {
            line!(
                shell,
                "  ● {} ({})",
                device.identifier(),
                device.abi().as_str()
            );
        }
    }

    if avds.is_empty() && connected_devices.is_empty() {
        line!(shell, "  No Android devices or emulators available");
    }
}

/// Display macOS device.
fn display_macos_devices(shell: &Shell) {
    header!(shell, "macOS");
    line!(shell, "  ● Current Machine");
}

/// Display ESP32 serial ports.
fn display_esp32_devices(shell: &Shell, ports: &[SerialPortSummary]) {
    header!(shell, "ESP32 (serial)");

    for port in ports {
        let marker = if port.likely_esp { "●" } else { "○" };
        let usb = port
            .usb_vid_pid
            .map(|(vid, pid)| format!(" [USB {vid:04x}:{pid:04x}]"))
            .unwrap_or_default();
        let product = port
            .product
            .as_deref()
            .map(|product| format!(" {product}"))
            .unwrap_or_default();
        let hint = if port.likely_esp { " (likely ESP)" } else { "" };
        line!(
            shell,
            "  {} {}{}{}{}",
            marker,
            port.port_name,
            usb,
            product,
            hint
        );
    }

    if ports.is_empty() {
        line!(shell, "  No serial ports available");
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    esp32: Option<Vec<JsonEsp32Device>>,
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

#[derive(Debug, Serialize)]
struct JsonEsp32Device {
    port: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    usb_vid: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usb_pid: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    product: Option<String>,
    likely_esp: bool,
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

fn json_esp32_devices(ports: &[SerialPortSummary]) -> Vec<JsonEsp32Device> {
    ports
        .iter()
        .map(|port| JsonEsp32Device {
            port: port.port_name.clone(),
            usb_vid: port.usb_vid_pid.map(|(vid, _)| vid),
            usb_pid: port.usb_vid_pid.map(|(_, pid)| pid),
            product: port.product.clone(),
            likely_esp: port.likely_esp,
        })
        .collect()
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
