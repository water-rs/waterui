use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};

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
    /// Tap at specific coordinates on the device screen
    Tap(TapArgs),
    /// Perform a swipe gesture on the device screen
    Swipe(SwipeArgs),
    /// Type text on the device (requires focused input field)
    Type(TypeArgs),
    /// Send a key event to the device
    Key(KeyArgs),
}

/// Platform filter for device commands
#[derive(ValueEnum, Copy, Clone, Debug)]
pub enum DevicePlatform {
    #[value(alias = "ios")]
    Apple,
    Android,
}

/// Cross-platform key names for key events
#[derive(ValueEnum, Copy, Clone, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyName {
    Home,
    Back,
    Enter,
    Tab,
    Escape,
    Delete,
    VolumeUp,
    VolumeDown,
    Power,
    Menu,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Args, Debug)]
pub struct CaptureArgs {
    /// Device to capture from (simulator name or device UDID)
    #[arg(long)]
    device: String,

    /// Platform to capture from (apple or android). Required if device name is ambiguous.
    #[arg(long)]
    platform: Option<DevicePlatform>,

    /// Output file path (defaults to ./screenshot.png)
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct TapArgs {
    /// X coordinate of the tap
    #[arg(long)]
    pub x: u32,

    /// Y coordinate of the tap
    #[arg(long)]
    pub y: u32,

    /// Device to interact with (simulator name or device UDID)
    #[arg(long)]
    pub device: String,

    /// Platform filter (apple or android)
    #[arg(long)]
    pub platform: Option<DevicePlatform>,

    /// Duration of the tap in milliseconds (for long press)
    #[arg(long)]
    pub duration: Option<u64>,

    /// Compare before/after screenshots. Optionally provide a path to save the visual diff image.
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub diff: Option<String>,
}

#[derive(Args, Debug)]
pub struct SwipeArgs {
    /// Starting X coordinate
    #[arg(long)]
    pub x1: u32,

    /// Starting Y coordinate
    #[arg(long)]
    pub y1: u32,

    /// Ending X coordinate
    #[arg(long)]
    pub x2: u32,

    /// Ending Y coordinate
    #[arg(long)]
    pub y2: u32,

    /// Device to interact with (simulator name or device UDID)
    #[arg(long)]
    pub device: String,

    /// Platform filter (apple or android)
    #[arg(long)]
    pub platform: Option<DevicePlatform>,

    /// Duration of the swipe in milliseconds
    #[arg(long, default_value = "300")]
    pub duration: u64,

    /// Compare before/after screenshots. Optionally provide a path to save the visual diff image.
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub diff: Option<String>,
}

#[derive(Args, Debug)]
pub struct TypeArgs {
    /// Text to type on the device
    pub text: String,

    /// Device to interact with (simulator name or device UDID)
    #[arg(long)]
    pub device: String,

    /// Platform filter (apple or android)
    #[arg(long)]
    pub platform: Option<DevicePlatform>,

    /// Compare before/after screenshots. Optionally provide a path to save the visual diff image.
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub diff: Option<String>,
}

#[derive(Args, Debug)]
pub struct KeyArgs {
    /// Key to send (e.g., home, back, enter, volume-up)
    pub key: KeyName,

    /// Device to interact with (simulator name or device UDID)
    #[arg(long)]
    pub device: String,

    /// Platform filter (apple or android)
    #[arg(long)]
    pub platform: Option<DevicePlatform>,

    /// Compare before/after screenshots. Optionally provide a path to save the visual diff image.
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub diff: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CaptureReport {
    pub device: String,
    pub platform: String,
    pub output_path: PathBuf,
}

waterui_cli::impl_report!(CaptureReport, |r| {
    format!("Screenshot saved to {}", r.output_path.display())
});

/// A rectangular region that changed between two screenshots
#[derive(Debug, Clone, Serialize)]
pub struct ChangedRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Result of comparing before/after screenshots
#[derive(Debug, Serialize)]
pub struct DiffResult {
    pub changed_rects: Vec<ChangedRect>,
    pub total_changed_pixels: u32,
    pub total_pixels: u32,
    pub change_percentage: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_image_path: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
pub struct TapReport {
    pub device: String,
    pub platform: String,
    pub x: u32,
    pub y: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<DiffResult>,
}

waterui_cli::impl_report!(TapReport, |r| {
    format!("Tapped at ({}, {}) on {}", r.x, r.y, r.device)
});

#[derive(Debug, Serialize)]
pub struct SwipeReport {
    pub device: String,
    pub platform: String,
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<DiffResult>,
}

waterui_cli::impl_report!(SwipeReport, |r| {
    format!("Swiped from {:?} to {:?} on {}", r.start, r.end, r.device)
});

#[derive(Debug, Serialize)]
pub struct TypeReport {
    pub device: String,
    pub platform: String,
    pub text_length: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<DiffResult>,
}

waterui_cli::impl_report!(TypeReport, |r| {
    format!("Typed {} characters on {}", r.text_length, r.device)
});

#[derive(Debug, Serialize)]
pub struct KeyReport {
    pub device: String,
    pub platform: String,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<DiffResult>,
}

waterui_cli::impl_report!(KeyReport, |r| {
    format!("Sent {} key to {}", r.key, r.device)
});

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
    platform_filter: Option<DevicePlatform>,
) -> Result<&'a DeviceInfo> {
    let matches: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|device| device.identifier == query || device.name == query)
        .filter(|device| {
            platform_filter.is_none_or(|filter| match filter {
                DevicePlatform::Apple => is_apple_platform(&device.platform),
                DevicePlatform::Android => device.platform == "Android",
            })
        })
        .collect();

    match matches.len() {
        0 => {
            let filter_hint = platform_filter
                .map(|p| format!(" for platform {p:?}"))
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
            let has_android = platforms.contains(&"Android");

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

// ============================================================================
// UI Automation: Tap
// ============================================================================

/// Send a tap gesture to the specified device.
///
/// # Errors
/// Returns an error if the device is not found, the platform doesn't support
/// UI automation, or the command execution fails.
pub fn tap(args: TapArgs) -> Result<TapReport> {
    let devices = device::list_devices().context("Failed to discover devices")?;

    if devices.is_empty() {
        bail!(
            "No devices found. Connect a device or start a simulator first.\n\
             Run `water devices` to see available targets."
        );
    }

    let device_info = find_device(&devices, &args.device, args.platform)?;

    // Check for real Apple device - UI automation not supported
    check_ui_automation_support(device_info)?;

    // Capture before screenshot if diff is requested
    let before_image = if args.diff.is_some() {
        Some(capture_temp_screenshot(device_info)?)
    } else {
        None
    };

    // Perform the tap
    match device_info.platform.as_str() {
        p if is_apple_platform(p) => {
            tap_apple_simulator(&device_info.identifier, args.x, args.y, args.duration)?;
        }
        "Android" => {
            tap_android(device_info, args.x, args.y, args.duration)?;
        }
        _ => {
            bail!(
                "Unsupported platform '{}' for tap.\n\
                 Supported platforms: iOS, iPadOS, watchOS, tvOS, visionOS, macOS, Android.",
                device_info.platform
            );
        }
    }

    // Capture after screenshot and compute diff if requested
    let diff_result = if let Some(before) = before_image {
        // Small delay to let UI settle
        thread::sleep(Duration::from_millis(300));
        let after = capture_temp_screenshot(device_info)?;
        Some(compute_and_report_diff(&before, &after, args.diff.as_deref())?)
    } else {
        None
    };

    Ok(TapReport {
        device: device_info.name.clone(),
        platform: device_info.platform.clone(),
        x: args.x,
        y: args.y,
        duration_ms: args.duration,
        diff: diff_result,
    })
}

// ============================================================================
// UI Automation: Swipe
// ============================================================================

/// Perform a swipe gesture on the specified device.
///
/// # Errors
/// Returns an error if the device is not found, the platform doesn't support
/// UI automation, or the command execution fails.
pub fn swipe(args: SwipeArgs) -> Result<SwipeReport> {
    let devices = device::list_devices().context("Failed to discover devices")?;

    if devices.is_empty() {
        bail!(
            "No devices found. Connect a device or start a simulator first.\n\
             Run `water devices` to see available targets."
        );
    }

    let device_info = find_device(&devices, &args.device, args.platform)?;
    check_ui_automation_support(device_info)?;

    let before_image = if args.diff.is_some() {
        Some(capture_temp_screenshot(device_info)?)
    } else {
        None
    };

    match device_info.platform.as_str() {
        p if is_apple_platform(p) => {
            swipe_apple_simulator(
                &device_info.identifier,
                args.x1,
                args.y1,
                args.x2,
                args.y2,
                args.duration,
            )?;
        }
        "Android" => {
            swipe_android(device_info, args.x1, args.y1, args.x2, args.y2, args.duration)?;
        }
        _ => {
            bail!(
                "Unsupported platform '{}' for swipe.\n\
                 Supported platforms: iOS, iPadOS, watchOS, tvOS, visionOS, macOS, Android.",
                device_info.platform
            );
        }
    }

    let diff_result = if let Some(before) = before_image {
        thread::sleep(Duration::from_millis(300));
        let after = capture_temp_screenshot(device_info)?;
        Some(compute_and_report_diff(&before, &after, args.diff.as_deref())?)
    } else {
        None
    };

    Ok(SwipeReport {
        device: device_info.name.clone(),
        platform: device_info.platform.clone(),
        start: (args.x1, args.y1),
        end: (args.x2, args.y2),
        duration_ms: args.duration,
        diff: diff_result,
    })
}

// ============================================================================
// UI Automation: Type Text
// ============================================================================

/// Type text on the specified device.
///
/// # Errors
/// Returns an error if the device is not found, the platform doesn't support
/// text input, or the command execution fails.
pub fn type_text(args: TypeArgs) -> Result<TypeReport> {
    let devices = device::list_devices().context("Failed to discover devices")?;

    if devices.is_empty() {
        bail!(
            "No devices found. Connect a device or start a simulator first.\n\
             Run `water devices` to see available targets."
        );
    }

    let device_info = find_device(&devices, &args.device, args.platform)?;
    check_ui_automation_support(device_info)?;

    let before_image = if args.diff.is_some() {
        Some(capture_temp_screenshot(device_info)?)
    } else {
        None
    };

    match device_info.platform.as_str() {
        p if is_apple_platform(p) => {
            type_apple_simulator(&device_info.identifier, &args.text)?;
        }
        "Android" => {
            type_android(device_info, &args.text)?;
        }
        _ => {
            bail!(
                "Unsupported platform '{}' for text input.\n\
                 Supported platforms: iOS, iPadOS, watchOS, tvOS, visionOS, macOS, Android.",
                device_info.platform
            );
        }
    }

    let diff_result = if let Some(before) = before_image {
        thread::sleep(Duration::from_millis(300));
        let after = capture_temp_screenshot(device_info)?;
        Some(compute_and_report_diff(&before, &after, args.diff.as_deref())?)
    } else {
        None
    };

    Ok(TypeReport {
        device: device_info.name.clone(),
        platform: device_info.platform.clone(),
        text_length: args.text.len(),
        diff: diff_result,
    })
}

// ============================================================================
// UI Automation: Key Event
// ============================================================================

/// Send a key event to the specified device.
///
/// # Errors
/// Returns an error if the device is not found, the key is not supported
/// on the platform, or the command execution fails.
pub fn key(args: KeyArgs) -> Result<KeyReport> {
    let devices = device::list_devices().context("Failed to discover devices")?;

    if devices.is_empty() {
        bail!(
            "No devices found. Connect a device or start a simulator first.\n\
             Run `water devices` to see available targets."
        );
    }

    let device_info = find_device(&devices, &args.device, args.platform)?;
    check_ui_automation_support(device_info)?;

    let before_image = if args.diff.is_some() {
        Some(capture_temp_screenshot(device_info)?)
    } else {
        None
    };

    match device_info.platform.as_str() {
        p if is_apple_platform(p) => {
            key_apple_simulator(&device_info.identifier, args.key)?;
        }
        "Android" => {
            key_android(device_info, args.key)?;
        }
        _ => {
            bail!(
                "Unsupported platform '{}' for key events.\n\
                 Supported platforms: iOS, iPadOS, watchOS, tvOS, visionOS, macOS, Android.",
                device_info.platform
            );
        }
    }

    let diff_result = if let Some(before) = before_image {
        thread::sleep(Duration::from_millis(300));
        let after = capture_temp_screenshot(device_info)?;
        Some(compute_and_report_diff(&before, &after, args.diff.as_deref())?)
    } else {
        None
    };

    Ok(KeyReport {
        device: device_info.name.clone(),
        platform: device_info.platform.clone(),
        key: format!("{:?}", args.key).to_lowercase(),
        diff: diff_result,
    })
}

// ============================================================================
// UI Automation Support Check
// ============================================================================

fn check_ui_automation_support(device_info: &DeviceInfo) -> Result<()> {
    if device_info.kind == DeviceKind::Device && is_apple_platform(&device_info.platform) {
        bail!(
            "UI automation is not supported on Apple physical devices.\n\
             This is a limitation imposed by Apple for security reasons.\n\
             \n\
             Alternatives:\n\
             • Use an iOS Simulator instead\n\
             • Use XCUITest for automated testing on physical devices\n\
             • Screenshot capture (water device capture) still works"
        );
    }
    Ok(())
}

// ============================================================================
// Android UI Automation Implementation
// ============================================================================

fn get_android_serial(device_info: &DeviceInfo) -> Result<String> {
    let adb_path = find_android_tool("adb").ok_or_else(|| {
        eyre!(
            "Android Debug Bridge (adb) not found.\n\
             Run `water doctor --fix` to install Android SDK tools."
        )
    })?;

    if device_info.kind == DeviceKind::Emulator {
        find_running_emulator_serial(&adb_path, &device_info.identifier)
    } else {
        Ok(device_info.identifier.clone())
    }
}

fn tap_android(device_info: &DeviceInfo, x: u32, y: u32, duration: Option<u64>) -> Result<()> {
    let adb_path = find_android_tool("adb").ok_or_else(|| {
        eyre!(
            "Android Debug Bridge (adb) not found.\n\
             Run `water doctor --fix` to install Android SDK tools."
        )
    })?;

    let serial = get_android_serial(device_info)?;

    // For long press, use swipe with same start/end
    let output = if let Some(dur) = duration.filter(|&d| d > 100) {
        Command::new(&adb_path)
            .args([
                "-s",
                &serial,
                "shell",
                "input",
                "swipe",
                &x.to_string(),
                &y.to_string(),
                &x.to_string(),
                &y.to_string(),
                &dur.to_string(),
            ])
            .output()
            .context("Failed to execute adb input swipe (long press)")?
    } else {
        Command::new(&adb_path)
            .args([
                "-s",
                &serial,
                "shell",
                "input",
                "tap",
                &x.to_string(),
                &y.to_string(),
            ])
            .output()
            .context("Failed to execute adb input tap")?
    };

    check_adb_output(&output, "tap")
}

fn swipe_android(
    device_info: &DeviceInfo,
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
    duration: u64,
) -> Result<()> {
    let adb_path = find_android_tool("adb").ok_or_else(|| {
        eyre!(
            "Android Debug Bridge (adb) not found.\n\
             Run `water doctor --fix` to install Android SDK tools."
        )
    })?;

    let serial = get_android_serial(device_info)?;

    let output = Command::new(&adb_path)
        .args([
            "-s",
            &serial,
            "shell",
            "input",
            "swipe",
            &x1.to_string(),
            &y1.to_string(),
            &x2.to_string(),
            &y2.to_string(),
            &duration.to_string(),
        ])
        .output()
        .context("Failed to execute adb input swipe")?;

    check_adb_output(&output, "swipe")
}

fn type_android(device_info: &DeviceInfo, text: &str) -> Result<()> {
    let adb_path = find_android_tool("adb").ok_or_else(|| {
        eyre!(
            "Android Debug Bridge (adb) not found.\n\
             Run `water doctor --fix` to install Android SDK tools."
        )
    })?;

    let serial = get_android_serial(device_info)?;

    // Escape special characters for shell
    let escaped = text
        .replace('\\', "\\\\")
        .replace(' ', "%s")
        .replace('&', "\\&")
        .replace('<', "\\<")
        .replace('>', "\\>")
        .replace('\'', "\\'")
        .replace('"', "\\\"");

    let output = Command::new(&adb_path)
        .args(["-s", &serial, "shell", "input", "text", &escaped])
        .output()
        .context("Failed to execute adb input text")?;

    check_adb_output(&output, "text input")
}

fn key_android(device_info: &DeviceInfo, key: KeyName) -> Result<()> {
    let adb_path = find_android_tool("adb").ok_or_else(|| {
        eyre!(
            "Android Debug Bridge (adb) not found.\n\
             Run `water doctor --fix` to install Android SDK tools."
        )
    })?;

    let serial = get_android_serial(device_info)?;

    let keycode = match key {
        KeyName::Home => "3",
        KeyName::Back => "4",
        KeyName::Enter => "66",
        KeyName::Tab => "61",
        KeyName::Escape => "111",
        KeyName::Delete => "67",
        KeyName::VolumeUp => "24",
        KeyName::VolumeDown => "25",
        KeyName::Power => "26",
        KeyName::Menu => "82",
        KeyName::Up => "19",
        KeyName::Down => "20",
        KeyName::Left => "21",
        KeyName::Right => "22",
    };

    let output = Command::new(&adb_path)
        .args(["-s", &serial, "shell", "input", "keyevent", keycode])
        .output()
        .context("Failed to execute adb input keyevent")?;

    check_adb_output(&output, "key event")
}

fn check_adb_output(output: &std::process::Output, action: &str) -> Result<()> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();

        if stderr.contains("device offline") {
            bail!(
                "Device is offline.\n\
                 Check the USB connection or restart the device."
            );
        } else if stderr.contains("device unauthorized") {
            bail!(
                "Device is unauthorized.\n\
                 Accept the USB debugging prompt on the device."
            );
        } else if !stderr.is_empty() {
            bail!("Failed to perform {}: {}", action, stderr);
        } else {
            bail!(
                "Failed to perform {} (exit code {}).",
                action,
                output.status.code().unwrap_or(-1)
            );
        }
    }
    Ok(())
}

// ============================================================================
// Apple Simulator UI Automation Implementation (via idb)
// ============================================================================

fn find_idb() -> Result<PathBuf> {
    which::which("idb").map_err(|_| {
        eyre!(
            "idb (iOS Development Bridge) not found.\n\
             UI automation on iOS simulators requires idb.\n\
             Run `water doctor --fix` to install it automatically."
        )
    })
}

fn tap_apple_simulator(device_id: &str, x: u32, y: u32, duration: Option<u64>) -> Result<()> {
    let idb = find_idb()?;

    let output = if let Some(dur) = duration.filter(|&d| d > 100) {
        // Long press - idb uses seconds as float
        let dur_secs = (dur as f64) / 1000.0;
        Command::new(&idb)
            .args([
                "ui",
                "tap",
                "--udid",
                device_id,
                "--duration",
                &dur_secs.to_string(),
                &x.to_string(),
                &y.to_string(),
            ])
            .output()
            .context("Failed to execute idb ui tap")?
    } else {
        Command::new(&idb)
            .args([
                "ui",
                "tap",
                "--udid",
                device_id,
                &x.to_string(),
                &y.to_string(),
            ])
            .output()
            .context("Failed to execute idb ui tap")?
    };

    check_idb_output(&output, "tap")
}

fn swipe_apple_simulator(
    device_id: &str,
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
    duration: u64,
) -> Result<()> {
    let idb = find_idb()?;

    let dur_secs = (duration as f64) / 1000.0;

    let output = Command::new(&idb)
        .args([
            "ui",
            "swipe",
            "--udid",
            device_id,
            "--duration",
            &dur_secs.to_string(),
            &x1.to_string(),
            &y1.to_string(),
            &x2.to_string(),
            &y2.to_string(),
        ])
        .output()
        .context("Failed to execute idb ui swipe")?;

    check_idb_output(&output, "swipe")
}

fn type_apple_simulator(device_id: &str, text: &str) -> Result<()> {
    let idb = find_idb()?;

    let output = Command::new(&idb)
        .args(["ui", "text", "--udid", device_id, text])
        .output()
        .context("Failed to execute idb ui text")?;

    check_idb_output(&output, "text input")
}

fn key_apple_simulator(device_id: &str, key: KeyName) -> Result<()> {
    let idb = find_idb()?;

    // idb uses different commands for hardware buttons vs keyboard keys
    let (cmd, key_arg) = match key {
        KeyName::Home => ("button", "HOME"),
        KeyName::Back => {
            bail!(
                "The 'back' key is not supported on iOS.\n\
                 iOS does not have a hardware back button.\n\
                 Use a swipe gesture instead: water device swipe --x1 0 --y1 400 --x2 200 --y2 400"
            );
        }
        KeyName::Enter => ("key", "13"),
        KeyName::Tab => ("key", "9"),
        KeyName::Escape => ("key", "27"),
        KeyName::Delete => ("key", "127"),
        KeyName::VolumeUp => ("button", "VOLUME_UP"),
        KeyName::VolumeDown => ("button", "VOLUME_DOWN"),
        KeyName::Power => ("button", "LOCK"),
        KeyName::Menu => {
            bail!(
                "The 'menu' key is not supported on iOS.\n\
                 This key is Android-only."
            );
        }
        KeyName::Up => ("key", "38"),
        KeyName::Down => ("key", "40"),
        KeyName::Left => ("key", "37"),
        KeyName::Right => ("key", "39"),
    };

    let output = Command::new(&idb)
        .args(["ui", cmd, "--udid", device_id, key_arg])
        .output()
        .context("Failed to execute idb ui key/button")?;

    check_idb_output(&output, "key event")
}

fn check_idb_output(output: &std::process::Output, action: &str) -> Result<()> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();

        if stderr.contains("not booted") || stderr.contains("not running") {
            bail!(
                "Simulator is not booted.\n\
                 Start the simulator first with `water run --device <name>`."
            );
        } else if stderr.contains("not found") {
            bail!(
                "Simulator not found.\n\
                 Run `water devices` to see available simulators."
            );
        } else if !stderr.is_empty() {
            bail!("Failed to perform {}: {}", action, stderr);
        } else {
            bail!(
                "Failed to perform {} (exit code {}).",
                action,
                output.status.code().unwrap_or(-1)
            );
        }
    }
    Ok(())
}

// ============================================================================
// Screenshot Diff Implementation
// ============================================================================

fn capture_temp_screenshot(device_info: &DeviceInfo) -> Result<DynamicImage> {
    let temp_path = std::env::temp_dir().join(format!(
        "waterui_screenshot_{}.png",
        std::process::id()
    ));

    match device_info.platform.as_str() {
        p if is_apple_platform(p) => {
            capture_apple_simulator(&device_info.identifier, &temp_path)?;
        }
        "Android" => {
            capture_android_device(device_info, &temp_path)?;
        }
        _ => bail!("Unsupported platform for screenshot"),
    }

    let img = image::open(&temp_path)
        .with_context(|| format!("Failed to read screenshot from {}", temp_path.display()))?;

    // Clean up temp file
    let _ = std::fs::remove_file(&temp_path);

    Ok(img)
}

fn compute_and_report_diff(
    before: &DynamicImage,
    after: &DynamicImage,
    output_path: Option<&str>,
) -> Result<DiffResult> {
    let (changed_rects, total_changed, total_pixels) = compute_diff(before, after);

    let change_percentage = if total_pixels > 0 {
        (f64::from(total_changed) / f64::from(total_pixels)) * 100.0
    } else {
        0.0
    };

    let diff_image_path = if let Some(path) = output_path {
        if path.is_empty() {
            None
        } else {
            let path = PathBuf::from(path);
            generate_diff_image(before, after, &path)?;
            Some(path)
        }
    } else {
        None
    };

    Ok(DiffResult {
        changed_rects,
        total_changed_pixels: total_changed,
        total_pixels,
        change_percentage,
        diff_image_path,
    })
}

fn compute_diff(before: &DynamicImage, after: &DynamicImage) -> (Vec<ChangedRect>, u32, u32) {
    let (width, height) = before.dimensions();
    let (after_width, after_height) = after.dimensions();

    // If dimensions don't match, return a single rect covering everything
    if width != after_width || height != after_height {
        return (
            vec![ChangedRect {
                x: 0,
                y: 0,
                width: after_width,
                height: after_height,
            }],
            after_width * after_height,
            after_width * after_height,
        );
    }

    let before_rgba = before.to_rgba8();
    let after_rgba = after.to_rgba8();

    let mut changed_pixels: Vec<(u32, u32)> = Vec::new();

    for y in 0..height {
        for x in 0..width {
            let before_pixel = before_rgba.get_pixel(x, y);
            let after_pixel = after_rgba.get_pixel(x, y);
            if before_pixel != after_pixel {
                changed_pixels.push((x, y));
            }
        }
    }

    let total_changed = changed_pixels.len() as u32;
    let total_pixels = width * height;

    // Group changed pixels into rectangular regions (simple bounding box approach)
    let rects = if changed_pixels.is_empty() {
        Vec::new()
    } else {
        // Find bounding box of all changed pixels
        let min_x = changed_pixels.iter().map(|(x, _)| *x).min().unwrap_or(0);
        let max_x = changed_pixels.iter().map(|(x, _)| *x).max().unwrap_or(0);
        let min_y = changed_pixels.iter().map(|(_, y)| *y).min().unwrap_or(0);
        let max_y = changed_pixels.iter().map(|(_, y)| *y).max().unwrap_or(0);

        vec![ChangedRect {
            x: min_x,
            y: min_y,
            width: max_x - min_x + 1,
            height: max_y - min_y + 1,
        }]
    };

    (rects, total_changed, total_pixels)
}

fn generate_diff_image(before: &DynamicImage, after: &DynamicImage, output: &Path) -> Result<()> {
    let (width, height) = after.dimensions();
    let before_rgba = before.to_rgba8();
    let after_rgba = after.to_rgba8();

    let mut diff = RgbaImage::new(width, height);

    for y in 0..height {
        for x in 0..width {
            let before_pixel = if x < before.width() && y < before.height() {
                *before_rgba.get_pixel(x, y)
            } else {
                Rgba([0, 0, 0, 255])
            };
            let after_pixel = *after_rgba.get_pixel(x, y);

            if before_pixel == after_pixel {
                // Dim unchanged pixels
                let dimmed = Rgba([
                    after_pixel[0] / 2,
                    after_pixel[1] / 2,
                    after_pixel[2] / 2,
                    after_pixel[3],
                ]);
                diff.put_pixel(x, y, dimmed);
            } else {
                // Highlight changed pixels in magenta
                diff.put_pixel(x, y, Rgba([255, 0, 255, 255]));
            }
        }
    }

    diff.save(output)
        .with_context(|| format!("Failed to save diff image to {}", output.display()))?;

    Ok(())
}
