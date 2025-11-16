use std::{path::Path, process::Command};

use color_eyre::eyre::{self, Context, Result};
use serde::Serialize;
use serde_json::Value;
use which::which;

use crate::{
    backend,
    platform::{AnyPlatform, Platform},
    project::{Project, RunOptions},
};

pub mod android;
pub mod apple;
pub use android::{AndroidDevice, AndroidSelection};
pub use apple::{AppleSimulatorDevice, MacosDevice};

pub trait Device: Send + Sync {
    type Platform: Platform;
    /// Perform any per-run setup (toolchain configuration, emulator launch, etc.).
    ///
    /// # Errors
    /// Returns an error if preparation fails.
    fn prepare(&self, project: &Project, options: &RunOptions) -> eyre::Result<()>;
    /// Run the packaged application artifact on this device.
    ///
    /// # Errors
    /// Returns an error if launching fails.
    fn run(&self, project: &Project, artifact: &Path, options: &RunOptions) -> eyre::Result<()>;
    fn platform(&self) -> &Self::Platform;
}

pub type AnyDevice = Box<dyn Device<Platform = AnyPlatform>>;

/// Scan for all available devices.
#[must_use]
pub fn scan() -> Vec<AnyDevice> {
    todo!("device scanning not implemented")
}

#[derive(Debug)]
pub struct LocalDevice;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DevicePlatformFilter {
    #[default]
    All,
    Apple,
    Android,
}

impl DevicePlatformFilter {
    const fn includes_apple(self) -> bool {
        matches!(self, Self::All | Self::Apple)
    }

    const fn includes_android(self) -> bool {
        matches!(self, Self::All | Self::Android)
    }
}

/// Collect a combined view of connected simulators and devices.
///
/// # Errors
/// Returns an error if querying connected devices fails.
pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    list_devices_filtered(DevicePlatformFilter::All)
}

/// Collect devices filtered by platform family.
///
/// # Errors
/// Returns an error if querying connected devices fails.
pub fn list_devices_filtered(filter: DevicePlatformFilter) -> Result<Vec<DeviceInfo>> {
    let mut devices = Vec::new();
    if filter.includes_apple() {
        devices.extend(apple_devices()?);
    }
    if filter.includes_android() {
        devices.extend(android_devices()?);
    }
    Ok(devices)
}

fn apple_devices() -> Result<Vec<DeviceInfo>> {
    if !cfg!(target_os = "macos") {
        return Ok(Vec::new());
    }
    if which("xcrun").is_err() {
        return Ok(Vec::new());
    }

    let output = Command::new("xcrun")
        .args(["xcdevice", "list", "--timeout=1"])
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
                .map(std::string::ToString::to_string);
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
                .map(std::string::ToString::to_string);

            let detail = item.get("error").map_or(operating_system, |error| {
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
            });

            let platform_label = raw_platform
                .as_deref()
                .map_or_else(|| "Apple".to_string(), apple_platform_friendly_name);

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

    if let Some(adb) = backend::android::find_android_tool("adb") {
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

    if let Some(emulator) = backend::android::find_android_tool("emulator") {
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
