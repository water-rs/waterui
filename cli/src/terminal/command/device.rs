use std::{
    fs::File,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

use clap::{Args, Subcommand, ValueEnum};
use color_eyre::eyre::{Context, Result, bail, eyre};
use serde::Serialize;
use waterui_cli::{
    backend::android::find_android_tool,
    device::{self, DeviceInfo, DeviceKind},
};

#[derive(Subcommand, Debug)]
pub enum DeviceCommands {
    /// Capture a screenshot from a simulator or device
    Capture(CaptureArgs),
}

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
pub fn capture(args: CaptureArgs) -> Result<CaptureReport> {
    let devices = device::list_devices().context("Failed to discover devices")?;

    if devices.is_empty() {
        bail!(
            "No devices found. Connect a device or start a simulator first.\n\
             Run `water devices` to see available targets."
        );
    }

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
            capture_apple_simulator(&device_info.identifier, &output_path)
                .with_context(|| format!("Failed to capture screenshot from Apple device '{}'", device_info.name))?;
        }
        "Android" => {
            capture_android_device(device_info, &output_path)
                .with_context(|| format!("Failed to capture screenshot from Android device '{}'", device_info.name))?;
        }
        _ => {
            bail!(
                "Unsupported platform '{}' for screenshot capture.\n\
                 Supported platforms: iOS, iPadOS, watchOS, tvOS, visionOS, macOS, Android.",
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

            // Provide helpful suggestions based on available devices
            let available_platforms: Vec<&str> = devices
                .iter()
                .map(|d| d.platform.as_str())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();

            let suggestion = if available_platforms.is_empty() {
                "No devices are currently available.".to_string()
            } else {
                format!(
                    "Available platforms: {}",
                    available_platforms.join(", ")
                )
            };

            Err(eyre!(
                "Device '{}' not found{}.\n\
                 {}\n\
                 Run `water devices` to see all available targets.",
                query,
                filter_hint,
                suggestion
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
                    "Device '{}' exists on multiple platforms (Apple and Android).\n\
                     Use --platform apple or --platform android to disambiguate.",
                    query
                ))
            } else {
                // Multiple devices on same platform - just use the first one
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
        .ok_or_else(|| eyre!("Output path contains invalid UTF-8 characters"))?;

    let output = Command::new("xcrun")
        .args(["simctl", "io", device_identifier, "screenshot", output_str])
        .output()
        .context("Failed to execute xcrun simctl. Is Xcode installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();

        // Provide more specific error messages based on common failure modes
        if stderr.contains("Invalid device") || stderr.contains("No matching device") {
            bail!(
                "Device '{}' is not available.\n\
                 The simulator may not be booted. Start it with `water run --device {}`.",
                device_identifier,
                device_identifier
            );
        } else if stderr.contains("Unable to") {
            bail!(
                "Unable to capture screenshot: {}\n\
                 Ensure the simulator is fully booted and responsive.",
                stderr
            );
        } else if !stderr.is_empty() {
            bail!("Screenshot capture failed: {}", stderr);
        } else {
            bail!(
                "Screenshot capture failed with exit code {}.\n\
                 Ensure the simulator is booted and responsive.",
                output.status.code().unwrap_or(-1)
            );
        }
    }

    Ok(())
}

fn capture_android_device(device_info: &DeviceInfo, output_path: &PathBuf) -> Result<()> {
    let adb_path = find_android_tool("adb").ok_or_else(|| {
        eyre!(
            "Android Debug Bridge (adb) not found.\n\
             Install the Android SDK platform-tools and ensure they are on your PATH.\n\
             You can install them via Android Studio or run:\n\
               brew install android-platform-tools  (macOS)\n\
               sudo apt install adb                 (Linux)"
        )
    })?;

    // For emulators, we need to check if the emulator is running
    // The identifier for emulators is the AVD name, but adb uses the serial number
    let serial = if device_info.kind == DeviceKind::Emulator {
        find_running_emulator_serial(&adb_path, &device_info.identifier)?
    } else {
        device_info.identifier.clone()
    };

    let output = Command::new(&adb_path)
        .args(["-s", &serial, "exec-out", "screencap", "-p"])
        .stdout(Stdio::piped())
        .output()
        .context("Failed to execute adb screencap command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();

        if stderr.contains("device offline") {
            bail!(
                "Device '{}' is offline.\n\
                 Check the USB connection or restart the device.",
                device_info.name
            );
        } else if stderr.contains("device unauthorized") {
            bail!(
                "Device '{}' is unauthorized.\n\
                 Accept the USB debugging prompt on the device.",
                device_info.name
            );
        } else if stderr.contains("not found") {
            bail!(
                "Device '{}' not found.\n\
                 The device may have been disconnected. Run `water devices` to check.",
                device_info.name
            );
        } else if !stderr.is_empty() {
            bail!("Screenshot capture failed: {}", stderr);
        } else {
            bail!(
                "Screenshot capture failed with exit code {}.",
                output.status.code().unwrap_or(-1)
            );
        }
    }

    if output.stdout.is_empty() {
        bail!(
            "Screenshot capture returned empty data.\n\
             The device screen may be locked or the display is off."
        );
    }

    let mut file = File::create(output_path)
        .with_context(|| format!("Failed to create output file '{}'", output_path.display()))?;

    file.write_all(&output.stdout)
        .with_context(|| format!("Failed to write screenshot to '{}'", output_path.display()))?;

    Ok(())
}

fn find_running_emulator_serial(adb_path: &PathBuf, avd_name: &str) -> Result<String> {
    let output = Command::new(adb_path)
        .args(["devices", "-l"])
        .output()
        .context("Failed to list connected Android devices")?;

    if !output.status.success() {
        bail!(
            "Failed to list Android devices.\n\
             Ensure the ADB server is running: `adb start-server`"
        );
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
        "Emulator '{}' is not running.\n\
         Start the emulator first with `water run --device {}`.",
        avd_name,
        avd_name
    );
}
