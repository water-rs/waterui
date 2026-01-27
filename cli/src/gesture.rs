//! Unified gesture API for device automation.
//!
//! Provides a platform-agnostic interface for performing gestures (tap, swipe, text)
//! on iOS simulators, Android devices, and macOS local machine.

use std::time::Duration;

use color_eyre::eyre;

use crate::capture::{DevicePlatform, detect_platform};
use crate::diff::DiffResult;
use crate::{android, apple};

/// Special device ID for macOS local machine.
pub const LOCAL_DEVICE_ID: &str = "local";

/// Options for gesture execution with diff support.
#[derive(Debug, Clone, Default)]
pub struct GestureOptions {
    /// Whether to capture before/after screenshots and compute diff.
    pub diff: bool,
    /// Path to save the diff image (only used if diff is true).
    pub diff_output: Option<std::path::PathBuf>,
    /// Delay in milliseconds after gesture before capturing "after" screenshot.
    /// Default is 500ms.
    pub delay_ms: Option<u32>,
}

impl GestureOptions {
    /// Create new gesture options with diff enabled.
    #[must_use]
    pub fn with_diff() -> Self {
        Self {
            diff: true,
            ..Default::default()
        }
    }

    /// Set the diff output path.
    #[must_use]
    pub fn diff_output(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.diff_output = Some(path.into());
        self
    }

    /// Set the delay after gesture in milliseconds.
    #[must_use]
    pub const fn delay(mut self, ms: u32) -> Self {
        self.delay_ms = Some(ms);
        self
    }
}

/// Result of a gesture with diff information.
#[derive(Debug)]
pub struct GestureResult {
    /// The diff result if diff was enabled.
    pub diff: Option<DiffResult>,
}

/// Capture screenshot bytes for a device.
async fn capture_screenshot_bytes(device_id: &str) -> eyre::Result<Vec<u8>> {
    if device_id == LOCAL_DEVICE_ID {
        apple::local::screenshot_bytes().await
    } else {
        match detect_platform(device_id) {
            DevicePlatform::Ios => apple::device::screenshot_bytes(device_id).await,
            DevicePlatform::Android => android::device::screenshot_bytes(device_id).await,
        }
    }
}

/// Execute a gesture with optional diff capture.
async fn execute_with_diff<F, Fut>(
    device_id: &str,
    options: &GestureOptions,
    gesture_fn: F,
) -> eyre::Result<GestureResult>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = eyre::Result<()>>,
{
    if !options.diff {
        // No diff, just execute the gesture
        gesture_fn().await?;
        return Ok(GestureResult { diff: None });
    }

    // Capture before screenshot
    let before = capture_screenshot_bytes(device_id).await?;

    // Execute the gesture
    gesture_fn().await?;

    // Wait for the specified delay (default 500ms)
    let delay = options.delay_ms.unwrap_or(500);
    smol::Timer::after(Duration::from_millis(u64::from(delay))).await;

    // Capture after screenshot
    let after = capture_screenshot_bytes(device_id).await?;

    // Compute diff
    let diff_result = crate::diff::compare_images(&before, &after)?;

    // Save diff image if requested
    if let Some(ref output_path) = options.diff_output {
        crate::diff::save_diff_image(&before, &after, output_path)?;
    }

    Ok(GestureResult {
        diff: Some(diff_result),
    })
}

/// Perform a tap gesture on a device.
///
/// # Arguments
///
/// * `device_id` - Device identifier (UDID for iOS, serial for Android, "local" for macOS)
/// * `x` - X coordinate
/// * `y` - Y coordinate
/// * `options` - Gesture options including diff settings
///
/// # Errors
///
/// Returns an error if the tap fails or the device is not available.
pub async fn tap(
    device_id: &str,
    x: u32,
    y: u32,
    options: &GestureOptions,
) -> eyre::Result<GestureResult> {
    execute_with_diff(device_id, options, || async {
        if device_id == LOCAL_DEVICE_ID {
            apple::local::tap(x, y).await
        } else {
            match detect_platform(device_id) {
                DevicePlatform::Ios => apple::device::tap(device_id, x, y).await,
                DevicePlatform::Android => android::device::tap(device_id, x, y).await,
            }
        }
    })
    .await
}

/// Perform a swipe gesture on a device.
///
/// # Arguments
///
/// * `device_id` - Device identifier
/// * `from` - Starting coordinates (x, y)
/// * `to` - Ending coordinates (x, y)
/// * `duration_ms` - Duration of the swipe in milliseconds
/// * `options` - Gesture options including diff settings
///
/// # Errors
///
/// Returns an error if the swipe fails or the device is not available.
pub async fn swipe(
    device_id: &str,
    from: (u32, u32),
    to: (u32, u32),
    duration_ms: Option<u32>,
    options: &GestureOptions,
) -> eyre::Result<GestureResult> {
    execute_with_diff(device_id, options, || async {
        if device_id == LOCAL_DEVICE_ID {
            apple::local::swipe(from, to, duration_ms).await
        } else {
            match detect_platform(device_id) {
                DevicePlatform::Ios => apple::device::swipe(device_id, from, to, duration_ms).await,
                DevicePlatform::Android => {
                    android::device::swipe(device_id, from, to, duration_ms).await
                }
            }
        }
    })
    .await
}

/// Input text on a device.
///
/// # Arguments
///
/// * `device_id` - Device identifier
/// * `input` - Text to input
/// * `options` - Gesture options including diff settings
///
/// # Errors
///
/// Returns an error if the text input fails or the device is not available.
pub async fn text(
    device_id: &str,
    input: &str,
    options: &GestureOptions,
) -> eyre::Result<GestureResult> {
    execute_with_diff(device_id, options, || async {
        if device_id == LOCAL_DEVICE_ID {
            apple::local::text(input).await
        } else {
            match detect_platform(device_id) {
                DevicePlatform::Ios => apple::device::text(device_id, input).await,
                DevicePlatform::Android => android::device::text(device_id, input).await,
            }
        }
    })
    .await
}

/// Verify that a device exists and is available.
///
/// # Errors
///
/// Returns an error if the device is not found or not available.
pub async fn verify_device(device_id: &str) -> eyre::Result<DevicePlatform> {
    if device_id == LOCAL_DEVICE_ID {
        // Local device is always available on macOS
        #[cfg(target_os = "macos")]
        return Ok(DevicePlatform::Ios); // Use iOS platform type for local (AppleScript-based)

        #[cfg(not(target_os = "macos"))]
        return Err(eyre::eyre!("Local device is only available on macOS"));
    }

    crate::capture::verify_device(device_id).await
}
