use std::{
    fs::File,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

use clap::{Args, ValueEnum};
use color_eyre::eyre::{Context, Result, bail, eyre};
use serde::Serialize;
use waterui_cli::{
    backend::android::find_android_tool,
    device::{self, DeviceInfo, DeviceKind},
};

#[derive(ValueEnum, Copy, Clone, Debug)]
pub enum CapturePlatform {
    #[value(alias = "ios")]
    Apple,
    Android,
}

#[derive(Args, Debug)]
pub struct CaptureArgs {
    /// Device to capture from (simulator name or device UDID)
    #[arg(long)]
    device: String,

    /// Platform to capture from (apple or android). Required if device name is ambiguous.
    #[arg(long)]
    platform: Option<CapturePlatform>,

    /// Output file path (defaults to ./screenshot.png)
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
pub struct CaptureReport {
    pub device: String,
    pub platform: String,
    pub output_path: PathBuf,
}

/// Capture a screenshot from the specified device/simulator.
///
/// # Errors
/// Returns an error if the device is not found, or if the screenshot capture fails.
pub fn run(args: CaptureArgs) -> Result<CaptureReport> {
    let devices = device::list_devices()?;
    let device_info = find_device(&devices, &args.device, args.platform)?;

    let output_path = args
        .output
        .unwrap_or_else(|| PathBuf::from("screenshot.png"));

    match device_info.platform.as_str() {
        p if p.starts_with("iOS")
            || p.starts_with("iPadOS")
            || p.starts_with("watchOS")
            || p.starts_with("tvOS")
            || p.starts_with("visionOS")
            || p == "macOS" =>
        {
            capture_apple_simulator(&device_info.identifier, &output_path)?;
        }
        "Android" => {
            capture_android_device(device_info, &output_path)?;
        }
        _ => {
            bail!(
                "Unsupported platform '{}' for screenshot capture",
                device_info.platform
            );
        }
    }

    Ok(CaptureReport {
        device: device_info.name.clone(),
        platform: device_info.platform.clone(),
        output_path,
    })
}

fn find_device<'a>(
    devices: &'a [DeviceInfo],
    query: &str,
    platform_filter: Option<CapturePlatform>,
) -> Result<&'a DeviceInfo> {
    let matches: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|device| device.identifier == query || device.name == query)
        .filter(|device| {
            platform_filter.map_or(true, |filter| match filter {
                CapturePlatform::Apple => is_apple_platform(&device.platform),
                CapturePlatform::Android => device.platform == "Android",
            })
        })
        .collect();

    match matches.len() {
        0 => {
            let filter_hint = platform_filter
                .map(|p| format!(" for platform {:?}", p))
                .unwrap_or_default();
            Err(eyre!(
                "Device '{}' not found{}. Run `water devices` to list available targets.",
                query,
                filter_hint
            ))
        }
        1 => Ok(matches[0]),
        _ => {
            // Multiple matches - check if they're on different platforms
            let platforms: Vec<&str> = matches.iter().map(|d| d.platform.as_str()).collect();
            let has_apple = platforms.iter().any(|p| is_apple_platform(p));
            let has_android = platforms.iter().any(|p| *p == "Android");

            if has_apple && has_android {
                Err(eyre!(
                    "Device '{}' exists on multiple platforms (Apple and Android). \
                    Use --platform apple or --platform android to disambiguate.",
                    query
                ))
            } else {
                // Multiple devices on same platform - just use the first one
                // (e.g., multiple iOS simulators with same name is unlikely but possible)
                Ok(matches[0])
            }
        }
    }
}

fn is_apple_platform(platform: &str) -> bool {
    platform.starts_with("iOS")
        || platform.starts_with("iPadOS")
        || platform.starts_with("watchOS")
        || platform.starts_with("tvOS")
        || platform.starts_with("visionOS")
        || platform == "macOS"
}

fn capture_apple_simulator(device_identifier: &str, output_path: &PathBuf) -> Result<()> {
    let output_str = output_path
        .to_str()
        .ok_or_else(|| eyre!("Output path is not valid UTF-8"))?;

    let status = Command::new("xcrun")
        .args(["simctl", "io", device_identifier, "screenshot", output_str])
        .status()
        .context("failed to execute xcrun simctl io screenshot")?;

    if !status.success() {
        bail!(
            "Failed to capture screenshot from simulator {}",
            device_identifier
        );
    }

    Ok(())
}

fn capture_android_device(device_info: &DeviceInfo, output_path: &PathBuf) -> Result<()> {
    let adb_path = find_android_tool("adb").ok_or_else(|| {
        eyre!(
            "`adb` not found. Install the Android SDK platform-tools and ensure they are available."
        )
    })?;

    // For emulators, we need to check if the emulator is running
    // The identifier for emulators is the AVD name, but adb uses the serial number
    let serial = if device_info.kind == DeviceKind::Emulator {
        // Find the running emulator's serial number
        find_running_emulator_serial(&adb_path, &device_info.identifier)?
    } else {
        device_info.identifier.clone()
    };

    let output = Command::new(&adb_path)
        .args(["-s", &serial, "exec-out", "screencap", "-p"])
        .stdout(Stdio::piped())
        .output()
        .context("failed to execute adb screencap")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "Failed to capture screenshot from Android device {}: {}",
            device_info.name,
            stderr.trim()
        );
    }

    let mut file = File::create(output_path)
        .with_context(|| format!("failed to create output file {}", output_path.display()))?;
    file.write_all(&output.stdout)
        .with_context(|| format!("failed to write screenshot to {}", output_path.display()))?;

    Ok(())
}

fn find_running_emulator_serial(adb_path: &PathBuf, avd_name: &str) -> Result<String> {
    // First, list all connected devices
    let output = Command::new(adb_path)
        .args(["devices", "-l"])
        .output()
        .context("failed to execute adb devices")?;

    if !output.status.success() {
        bail!("Failed to list Android devices");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Look for emulator devices (they have serial numbers like emulator-5554)
    for line in stdout.lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        let serial = parts[0];
        if !serial.starts_with("emulator-") {
            continue;
        }

        // Check if this emulator matches the AVD name
        let avd_output = Command::new(adb_path)
            .args(["-s", serial, "emu", "avd", "name"])
            .output();

        if let Ok(avd_output) = avd_output {
            let name = String::from_utf8_lossy(&avd_output.stdout);
            let name = name.trim().lines().next().unwrap_or("");
            if name == avd_name {
                return Ok(serial.to_string());
            }
        }
    }

    bail!(
        "Emulator '{}' is not running. Start the emulator first with `water run --device {}`.",
        avd_name,
        avd_name
    );
}
