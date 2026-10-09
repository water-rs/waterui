//! `water devices` command implementation.

use std::{collections::HashSet, path::PathBuf};

use clap::{Args as ClapArgs, ValueEnum};
use eyre::{Result, bail};
use serde::Serialize;

use crate::shell::Shell;
use crate::{header, line};
use smol::future::zip;
use waterui_cli::{
    android::{
        AndroidSdk,
        adb::Adb,
        device::{AndroidDevice, emulator_avd_name_with_adb},
    },
    apple::{device::AppleSimulator, physical::ApplePhysicalDevice},
    toolchain::Host,
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
pub async fn run(shell: &Shell, args: Args) -> Result<()> {
    if shell.is_json() {
        return run_json(shell, args).await;
    }

    let host = Host::current();
    match args.platform {
        TargetPlatform::Ios => {
            let (physical, sims) = scan_ios_devices(&host).await?;
            display_ios_devices(shell, &physical, &sims);
        }
        TargetPlatform::Android => {
            let adb = Adb::locate(&host).await?;
            let (avds, devices, running_avds) =
                scan_android_devices(&host, AndroidSdk::emulator_path(&host), adb).await?;
            display_android_devices(shell, &avds, &devices, &running_avds);
        }
        TargetPlatform::Macos => {
            display_macos_devices(shell);
        }
        TargetPlatform::All => {
            // Fast-fail only when adb is unavailable.
            let adb = Adb::locate(&host).await?;
            let spinner = shell.spinner("Scanning devices...");

            // Scan iOS and Android in parallel
            let (ios_devices, android_result) = zip(
                scan_ios_devices(&host),
                scan_android_devices(&host, AndroidSdk::emulator_path(&host), adb),
            )
            .await;

            if let Some(pb) = spinner {
                pb.finish_and_clear();
            }

            // Display results in order
            let (physical, sims) = ios_devices?;
            display_ios_devices(shell, &physical, &sims);
            {
                let (avds, devices, running_avds) = android_result?;
                display_android_devices(shell, &avds, &devices, &running_avds);
            }
            display_macos_devices(shell);
        }
    }

    Ok(())
}

async fn run_json(shell: &Shell, args: Args) -> Result<()> {
    let host = Host::current();
    let output = match args.platform {
        TargetPlatform::Ios => {
            let (physical, sims) = scan_ios_devices(&host).await?;
            DevicesJsonOutput {
                ty: "devices",
                platform: "ios",
                ios: Some(json_ios_devices(&physical, &sims)),
                android: None,
                macos: None,
            }
        }
        TargetPlatform::Android => {
            let adb = Adb::locate(&host).await?;
            let (avds, devices, running_avds) =
                scan_android_devices(&host, AndroidSdk::emulator_path(&host), adb).await?;
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
            let adb = Adb::locate(&host).await?;
            let (ios_devices, android_result) = zip(
                scan_ios_devices(&host),
                scan_android_devices(&host, AndroidSdk::emulator_path(&host), adb),
            )
            .await;
            let (physical, sims) = ios_devices?;
            let (avds, devices, running_avds) = android_result?;

            DevicesJsonOutput {
                ty: "devices",
                platform: "all",
                ios: Some(json_ios_devices(&physical, &sims)),
                android: Some(json_android_section(&avds, &devices, &running_avds)),
                macos: Some(vec![JsonMacosDevice {
                    id: "local".to_string(),
                    name: "Current Machine".to_string(),
                }]),
            }
        }
    };

    let _ = shell.json_raw(&serde_json::to_string(&output)?);
    Ok(())
}

/// Scan iOS devices and simulators on `host`.
///
/// The `devicectl` scan is non-fatal — a machine without paired devices (or
/// without `CoreDevice`) still has its simulators to list.
async fn scan_ios_devices(host: &Host) -> Result<(Vec<ApplePhysicalDevice>, Vec<AppleSimulator>)> {
    let (physical, simulators) = zip(
        ApplePhysicalDevice::scan(host),
        AppleSimulator::scan_ios(host),
    )
    .await;
    let physical = physical.unwrap_or_else(|error| {
        tracing::warn!("devicectl device scan failed: {error:#}");
        Vec::new()
    });
    Ok((physical, simulators?))
}

/// Scan Android devices and emulators on `host`.
async fn scan_android_devices(
    host: &Host,
    emulator_path: Option<PathBuf>,
    adb: Adb,
) -> Result<(Vec<String>, Vec<AndroidDevice>, HashSet<String>)> {
    // List available AVDs (emulators) and connected devices in parallel
    let avds_future = async move {
        let Some(emulator_path) = emulator_path else {
            return Ok(Vec::new());
        };
        host.output(&emulator_path, ["-list-avds"])
            .await
            .map_err(|e| eyre::eyre!("Failed to list AVDs: {e}"))
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

    let devices_future = AndroidDevice::scan_with_adb(host, &adb);

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
        let name = emulator_avd_name_with_adb(host, &adb, id).await?;
        running_avds.insert(name);
    }

    Ok((avds, connected_devices, running_avds))
}

/// Display iOS devices: paired physical devices first, then simulators.
fn display_ios_devices(shell: &Shell, physical: &[ApplePhysicalDevice], devs: &[AppleSimulator]) {
    if !physical.is_empty() {
        header!(shell, "iOS Devices");
        for device in physical {
            let reachable = device.usability().is_ok();
            let state_icon = if reachable { "●" } else { "○" };
            let transport = match device.transport {
                waterui_cli::apple::physical::Transport::Wired => "USB",
                waterui_cli::apple::physical::Transport::LocalNetwork => "Wi-Fi",
                waterui_cli::apple::physical::Transport::Other => "?",
            };
            let os = device
                .os_version
                .as_ref()
                .map_or_else(String::new, |v| format!(" — iOS {v}"));
            line!(
                shell,
                "  {} {} [{}]{} ({})",
                state_icon,
                device.name,
                transport,
                os,
                device.identifier
            );
        }
    }

    if !devs.is_empty() {
        header!(shell, "iOS Simulators");
    }

    for sim in devs {
        let state_icon = if sim.state == "Booted" { "●" } else { "○" };
        line!(shell, "  {} {} ({})", state_icon, sim.name, sim.udid);
    }

    if devs.is_empty() && physical.is_empty() {
        line!(shell, "  No iOS simulators or devices available");
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
    /// `"device"` for a paired physical device, `"simulator"` otherwise.
    kind: &'static str,
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

fn json_ios_devices(
    physical: &[ApplePhysicalDevice],
    devices: &[AppleSimulator],
) -> Vec<JsonIosDevice> {
    physical
        .iter()
        .map(|device| JsonIosDevice {
            name: device.name.clone(),
            udid: device.udid.clone(),
            state: format!("{:?}", device.tunnel_state),
            available: device.usability().is_ok(),
            kind: "device",
        })
        .chain(devices.iter().map(|sim| JsonIosDevice {
            name: sim.name.clone(),
            udid: sim.udid.clone(),
            state: sim.state.clone(),
            available: sim.is_available,
            kind: "simulator",
        }))
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
            kind: "simulator",
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
