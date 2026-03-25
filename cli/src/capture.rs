//! Screen capture utilities for devices.
//!
//! Provides unified screenshot capture across iOS simulators and Android devices.

use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, eyre};
use jiff::Timestamp;

use crate::device::Device;
use crate::{android, apple};

/// The platform type of a device based on its identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicePlatform {
    /// iOS simulator (UDID format)
    Ios,
    /// Android device or emulator
    Android,
}

/// Detect the platform of a device from its identifier.
///
/// iOS simulators use UUIDs (e.g., `12345678-1234-1234-1234-123456789ABC`).
/// Android devices use serials (e.g., `emulator-5554`, `ABCD1234`).
#[must_use]
pub fn detect_platform(device_id: &str) -> DevicePlatform {
    let parts: Vec<&str> = device_id.split('-').collect();
    if parts.len() == 5
        && parts[0].len() == 8
        && parts[1].len() == 4
        && parts[2].len() == 4
        && parts[3].len() == 4
        && parts[4].len() == 12
        && device_id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
    {
        return DevicePlatform::Ios;
    }

    DevicePlatform::Android
}

/// Generate a default output filename for a screenshot.
///
/// Format: `screenshot_YYYY-MM-DD_HHMMSS.png`
#[must_use]
pub fn generate_screenshot_filename() -> PathBuf {
    let filename = format!(
        "screenshot_{}.png",
        Timestamp::now().strftime("%Y-%m-%d_%H%M%S")
    );
    PathBuf::from(filename)
}

/// Capture a screenshot from a device.
///
/// # Errors
/// Returns an error if the target device cannot be reached or the screenshot command fails.
pub async fn screenshot(device_id: &str, output: &Path) -> eyre::Result<()> {
    match detect_platform(device_id) {
        DevicePlatform::Ios => apple::device::screenshot(device_id, output).await,
        DevicePlatform::Android => android::device::screenshot(device_id, output).await,
    }
}

/// Verify that a device exists and return its platform.
///
/// # Errors
/// Returns an error if the device cannot be found on the inferred platform.
pub async fn verify_device(device_id: &str) -> eyre::Result<DevicePlatform> {
    let platform = detect_platform(device_id);

    match platform {
        DevicePlatform::Ios => {
            let simulators = apple::device::AppleSimulator::scan().await?;
            if simulators.iter().any(|s| s.udid == device_id) {
                Ok(DevicePlatform::Ios)
            } else {
                Err(eyre!("iOS simulator with UDID '{}' not found", device_id))
            }
        }
        DevicePlatform::Android => {
            let devices = android::device::AndroidDevice::scan().await?;
            if devices.iter().any(|d| d.identifier() == device_id) {
                Ok(DevicePlatform::Android)
            } else {
                Err(eyre!("Android device '{}' not found", device_id))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_ios_udid() {
        assert_eq!(
            detect_platform("12345678-1234-1234-1234-123456789ABC"),
            DevicePlatform::Ios
        );
        assert_eq!(
            detect_platform("ABCDEF12-3456-7890-ABCD-EF1234567890"),
            DevicePlatform::Ios
        );
    }

    #[test]
    fn detects_android_device() {
        assert_eq!(detect_platform("emulator-5554"), DevicePlatform::Android);
        assert_eq!(detect_platform("ABCD1234"), DevicePlatform::Android);
        assert_eq!(
            detect_platform("192.168.1.100:5555"),
            DevicePlatform::Android
        );
    }

    #[test]
    fn generates_valid_filename() {
        let filename = generate_screenshot_filename();
        let name = filename.to_string_lossy();
        assert!(name.starts_with("screenshot_"));
        assert!(name.ends_with(".png"));
    }
}
