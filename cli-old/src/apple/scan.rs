use color_eyre::eyre::{self, Context};
use serde::{Deserialize, Serialize};
use smol::process::Command;

#[derive(Debug, Serialize, Deserialize)]
struct DeviceInfo {
    platform: String,
    raw_platform: Option<String>,
    name: String,
    identifier: String,
    kind: DeviceKind,
    state: Option<String>,
    detail: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceError {
    description: Option<String>,
    #[serde(rename = "recoverySuggestion")]
    recovery_suggestion: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceRaw {
    name: Option<String>,
    identifier: Option<String>,

    platform: Option<String>,
    simulator: Option<bool>,
    available: Option<bool>,

    #[serde(rename = "operatingSystemVersion")]
    operating_system_version: Option<String>,

    error: Option<DeviceError>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DeviceKind {
    Simulator,
    Device,
}

async fn scan_inner() -> eyre::Result<Vec<DeviceRaw>> {
    let output = Command::new("xcrun")
        .args(["xcdevice", "list", "--timeout=1"])
        .kill_on_drop(true)
        .output()
        .await
        .context("failed to execute xcrun xcdevice list")?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let devices: Vec<DeviceRaw> = serde_json::from_slice(&output.stdout).unwrap_or_default();

    Ok(devices)
}
