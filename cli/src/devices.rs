use clap::Args;
use color_eyre::eyre::{Context, Result};
use console::style;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::process::Command;
use which::which;

#[derive(Args, Debug, Default)]
pub struct DevicesArgs {
    /// Output devices as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Simulator,
    Device,
    Emulator,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceInfo {
    pub platform: String,
    pub raw_platform: Option<String>,
    pub name: String,
    pub identifier: String,
    pub kind: DeviceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    let mut devices = Vec::new();
    devices.extend(apple_devices()?);
    devices.extend(android_devices()?);
    Ok(devices)
}

pub fn find_android_tool(tool: &str) -> Option<PathBuf> {
    if let Ok(path) = which(tool) {
        return Some(path);
    }

    let suffixes: &[&str] = match tool {
        "adb" => &["platform-tools/adb", "platform-tools/adb.exe"],
        "emulator" => &["emulator/emulator", "emulator/emulator.exe"],
        _ => &[],
    };

    for root in android_sdk_roots() {
        for suffix in suffixes {
            let candidate = root.join(suffix);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

pub fn run(args: DevicesArgs) -> Result<()> {
    let devices = list_devices()?;

    if devices.is_empty() {
        println!("No devices detected. Install a simulator or connect a device, then try again.");
        return Ok(());
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&devices)?);
    } else {
        print_table(&devices);
    }

    Ok(())
}

fn apple_devices() -> Result<Vec<DeviceInfo>> {
    if !cfg!(target_os = "macos") {
        return Ok(Vec::new());
    }
    if which("xcrun").is_err() {
        return Ok(Vec::new());
    }

    let output = Command::new("xcrun")
        .args(["xcdevice", "list"])
        .output()
        .context("failed to execute xcrun xcdevice list")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }

    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    let mut results = Vec::new();

    if let Some(array) = value.as_array() {
        for item in array {
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Unknown")
                .to_string();
            let identifier = item
                .get("identifier")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if identifier.is_empty() {
                continue;
            }

            let raw_platform = item
                .get("platform")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            let simulator = item
                .get("simulator")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let available = item
                .get("available")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let operating_system = item
                .get("operatingSystemVersion")
                .and_then(Value::as_str)
                .map(|s| s.to_string());

            let detail = if let Some(error) = item.get("error") {
                let description = error
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let suggestion = error
                    .get("recoverySuggestion")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut message = description.to_string();
                if !suggestion.is_empty() {
                    if !message.is_empty() {
                        message.push_str(" – ");
                    }
                    message.push_str(suggestion);
                }
                if message.is_empty() {
                    None
                } else {
                    Some(message)
                }
            } else {
                operating_system
            };

            let platform_label = raw_platform
                .as_deref()
                .map(apple_platform_friendly_name)
                .unwrap_or_else(|| "Apple".to_string());

            results.push(DeviceInfo {
                platform: platform_label,
                raw_platform,
                name,
                identifier,
                kind: if simulator {
                    DeviceKind::Simulator
                } else {
                    DeviceKind::Device
                },
                state: Some(if available {
                    "available".to_string()
                } else {
                    "unavailable".to_string()
                }),
                detail,
            });
        }
    }

    Ok(results)
}

fn android_devices() -> Result<Vec<DeviceInfo>> {
    let mut results = Vec::new();

    if let Some(adb) = find_android_tool("adb") {
        let output = Command::new(&adb)
            .args(["devices", "-l"])
            .output()
            .context("failed to execute adb devices")?;
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines().skip(1) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let mut parts = trimmed.split_whitespace();
                let identifier = parts.next().unwrap_or_default();
                let state = parts.next().unwrap_or_default();
                if identifier.is_empty() || identifier == "offline" {
                    continue;
                }

                let mut name = identifier.to_string();
                let mut detail_segments = Vec::new();
                for token in parts {
                    detail_segments.push(token.to_string());
                    if let Some(rest) = token.strip_prefix("model:") {
                        if !rest.is_empty() {
                            name = rest.replace('_', " ");
                        }
                    }
                }

                let detail = if detail_segments.is_empty() {
                    None
                } else {
                    Some(detail_segments.join(" "))
                };

                results.push(DeviceInfo {
                    platform: "Android".to_string(),
                    raw_platform: Some("android-device".to_string()),
                    name,
                    identifier: identifier.to_string(),
                    kind: DeviceKind::Device,
                    state: Some(state.to_string()),
                    detail,
                });
            }
        }
    }

    if let Some(emulator) = find_android_tool("emulator") {
        let output = Command::new(&emulator)
            .arg("-list-avds")
            .output()
            .context("failed to execute emulator -list-avds")?;
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let name = line.trim();
                if name.is_empty() {
                    continue;
                }
                results.push(DeviceInfo {
                    platform: "Android".to_string(),
                    raw_platform: Some("android-emulator".to_string()),
                    name: name.to_string(),
                    identifier: name.to_string(),
                    kind: DeviceKind::Emulator,
                    state: Some("stopped".to_string()),
                    detail: None,
                });
            }
        }
    }

    Ok(results)
}

fn apple_platform_friendly_name(identifier: &str) -> String {
    match identifier {
        "com.apple.platform.iphoneos" => "iOS device",
        "com.apple.platform.ipados" => "iPadOS device",
        "com.apple.platform.watchos" => "watchOS device",
        "com.apple.platform.appletvos" => "tvOS device",
        "com.apple.platform.iphonesimulator" => "iOS simulator",
        "com.apple.platform.appletvsimulator" => "tvOS simulator",
        "com.apple.platform.watchsimulator" => "watchOS simulator",
        "com.apple.platform.visionos" => "visionOS device",
        "com.apple.platform.visionossimulator" => "visionOS simulator",
        "com.apple.platform.macosx" => "macOS",
        other => other,
    }
    .to_string()
}

fn print_table(devices: &[DeviceInfo]) {
    let mut grouped: BTreeMap<String, Vec<&DeviceInfo>> = BTreeMap::new();
    for device in devices {
        grouped
            .entry(device.platform.clone())
            .or_default()
            .push(device);
    }

    for (idx, (platform, list)) in grouped.iter().enumerate() {
        if idx > 0 {
            println!();
        }
        println!("{}", style(platform).bold().underlined());

        let mut items: Vec<&DeviceInfo> = list.to_vec();
        items.sort_by(|a, b| {
            let kind_rank = |kind: &DeviceKind| match kind {
                DeviceKind::Device => 0,
                DeviceKind::Simulator => 1,
                DeviceKind::Emulator => 2,
            };
            kind_rank(&a.kind)
                .cmp(&kind_rank(&b.kind))
                .then_with(|| a.name.cmp(&b.name))
        });

        for device in items {
            let bullet = style("•").cyan();
            let name = style(&device.name).bold();
            let kind_label = match device.kind {
                DeviceKind::Device => style("device").green(),
                DeviceKind::Simulator => style("simulator").magenta(),
                DeviceKind::Emulator => style("emulator").yellow(),
            };
            let state_text =
                device
                    .state
                    .as_deref()
                    .unwrap_or(if device.kind == DeviceKind::Emulator {
                        "stopped"
                    } else {
                        "-"
                    });
            let state_label = match state_text {
                "available" | "device" | "online" => style(state_text).green(),
                "unavailable" | "offline" => style(state_text).red(),
                "stopped" => style(state_text).yellow(),
                other => style(other).dim(),
            };

            println!("  {} {} ({}, {})", bullet, name, kind_label, state_label);
            println!(
                "      {}",
                style(format!("id: {}", device.identifier)).dim()
            );
            if let Some(detail) = &device.detail {
                println!("      {}", style(detail).dim());
            }
        }
    }
}

fn android_sdk_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(path) = env::var("ANDROID_HOME") {
        roots.push(PathBuf::from(path));
    }
    if let Ok(path) = env::var("ANDROID_SDK_ROOT") {
        roots.push(PathBuf::from(path));
    }
    if let Ok(home) = env::var("HOME") {
        let home_path = PathBuf::from(home);
        roots.push(home_path.join("Library/Android/sdk"));
        roots.push(home_path.join("Android/Sdk"));
    }
    roots.into_iter().filter(|p| p.exists()).collect()
}
