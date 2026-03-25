//! macOS local device gestures and screenshot support.
//!
//! Provides gesture automation for the current macOS machine using AppleScript
//! and the `screencapture` command for screenshots.

use std::path::Path;
use std::time::Duration;

use color_eyre::eyre::{self, eyre};
use core_foundation::base::{CFType, TCFType};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::window::{
    CGWindowListCopyWindowInfo, kCGNullWindowID, kCGWindowListOptionOnScreenOnly,
};
use smol::process::Command;

/// Information about a macOS window.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    /// The window ID (used for screencapture -l).
    pub window_id: u32,
    /// The window title/name.
    pub name: String,
    /// The owning process ID.
    pub owner_pid: i32,
    /// The owning application name.
    pub owner_name: String,
    /// Window layer (0 = normal windows).
    pub layer: i32,
}

/// List all windows belonging to a specific process.
///
/// Uses Core Graphics `CGWindowListCopyWindowInfo` to enumerate windows.
///
/// # Arguments
///
/// * `pid` - The process ID to filter windows by
///
/// # Returns
///
/// A vector of `WindowInfo` for all windows owned by the process,
/// sorted by window layer (main windows first).
///
/// # Errors
/// Returns an error if Core Graphics window metadata cannot be queried.
pub fn list_windows_by_pid(pid: i32) -> eyre::Result<Vec<WindowInfo>> {
    use core_foundation::array::CFArray;
    use core_foundation::dictionary::CFDictionary;

    let window_list_ptr =
        unsafe { CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, kCGNullWindowID) };

    if window_list_ptr.is_null() {
        eyre::bail!("Failed to get window list from Core Graphics");
    }

    let window_list: CFArray<CFDictionary<CFString, CFType>> =
        unsafe { CFArray::wrap_under_create_rule(window_list_ptr) };

    let mut windows = Vec::new();

    for window_dict in window_list.iter() {
        // Get owner PID
        let owner_pid_key = CFString::new("kCGWindowOwnerPID");
        let Some(owner_pid_val) = window_dict.find(&owner_pid_key) else {
            continue;
        };
        let owner_pid_num =
            unsafe { CFNumber::wrap_under_get_rule(owner_pid_val.as_CFTypeRef().cast()) };
        let Some(owner_pid) = owner_pid_num.to_i32() else {
            continue;
        };

        // Filter by PID
        if owner_pid != pid {
            continue;
        }

        // Get window ID
        let window_id_key = CFString::new("kCGWindowNumber");
        let Some(window_id_val) = window_dict.find(&window_id_key) else {
            continue;
        };
        let window_id_num =
            unsafe { CFNumber::wrap_under_get_rule(window_id_val.as_CFTypeRef().cast()) };
        let Some(window_id) = window_id_num.to_i32() else {
            continue;
        };

        // Get window name (may be empty)
        let name_key = CFString::new("kCGWindowName");
        let name = window_dict
            .find(&name_key)
            .map(|v| {
                let cf_str = unsafe { CFString::wrap_under_get_rule(v.as_CFTypeRef().cast()) };
                cf_str.to_string()
            })
            .unwrap_or_default();

        // Get owner name
        let owner_name_key = CFString::new("kCGWindowOwnerName");
        let owner_name = window_dict
            .find(&owner_name_key)
            .map(|v| {
                let cf_str = unsafe { CFString::wrap_under_get_rule(v.as_CFTypeRef().cast()) };
                cf_str.to_string()
            })
            .unwrap_or_default();

        // Get window layer
        let layer_key = CFString::new("kCGWindowLayer");
        let layer = window_dict
            .find(&layer_key)
            .and_then(|v| {
                let num = unsafe { CFNumber::wrap_under_get_rule(v.as_CFTypeRef().cast()) };
                num.to_i32()
            })
            .unwrap_or(0);

        windows.push(WindowInfo {
            window_id: window_id.cast_unsigned(),
            name,
            owner_pid,
            owner_name,
            layer,
        });
    }

    // Sort by layer (layer 0 = normal windows, negative = below, positive = above)
    // Main windows typically have layer 0
    windows.sort_by_key(|w| w.layer);

    Ok(windows)
}

/// Capture a screenshot of a specific window by its window ID.
///
/// Uses `screencapture -l <windowid>` to capture a single window.
///
/// # Arguments
///
/// * `window_id` - The window ID (from `WindowInfo::window_id`)
/// * `output` - The output file path
///
/// # Errors
///
/// Returns an error if the screenshot fails.
pub async fn screenshot_window(window_id: u32, output: &Path) -> eyre::Result<()> {
    let output_str = output
        .to_str()
        .ok_or_else(|| eyre!("Invalid output path"))?;

    let result = Command::new("screencapture")
        .arg("-x") // No sound
        .arg("-l")
        .arg(window_id.to_string())
        .arg(output_str)
        .output()
        .await?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        eyre::bail!("Failed to capture window screenshot: {}", stderr.trim());
    }

    Ok(())
}

/// Capture a screenshot of a specific window and return the raw PNG bytes.
///
/// # Arguments
///
/// * `window_id` - The window ID (from `WindowInfo::window_id`)
///
/// # Errors
///
/// Returns an error if the screenshot fails.
pub async fn screenshot_window_bytes(window_id: u32) -> eyre::Result<Vec<u8>> {
    let result = Command::new("screencapture")
        .arg("-x") // No sound
        .arg("-l")
        .arg(window_id.to_string())
        .arg("-t")
        .arg("png")
        .arg("-") // Output to stdout
        .output()
        .await?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        eyre::bail!("Failed to capture window screenshot: {}", stderr.trim());
    }

    Ok(result.stdout)
}

/// Perform a tap (click) gesture at the specified screen coordinates.
///
/// Uses `AppleScript` to click at the given absolute screen position.
///
/// # Arguments
///
/// * `x` - X coordinate on screen
/// * `y` - Y coordinate on screen
///
/// # Errors
///
/// Returns an error if the click fails.
pub async fn tap(x: u32, y: u32) -> eyre::Result<()> {
    let script = format!(
        include_str!("applescript/tap.applescript.tpl"),
        x = x,
        y = y,
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eyre::bail!("Failed to tap: {}", stderr.trim());
    }

    // Small delay to let the click register
    smol::Timer::after(Duration::from_millis(100)).await;

    Ok(())
}

/// Perform a swipe (drag) gesture on macOS.
///
/// Uses `AppleScript` to simulate a drag from one point to another.
/// Note: `AppleScript`'s built-in drag is limited; this uses a click-based approximation.
///
/// # Arguments
///
/// * `from` - Starting coordinates (x, y)
/// * `to` - Ending coordinates (x, y)
/// * `duration_ms` - Duration of the swipe in milliseconds
///
/// # Errors
///
/// Returns an error if the swipe fails.
pub async fn swipe(from: (u32, u32), to: (u32, u32), duration_ms: Option<u32>) -> eyre::Result<()> {
    let duration_sec = f64::from(duration_ms.unwrap_or(300)) / 1000.0;

    // AppleScript doesn't have native drag support
    // We simulate with click at start, delay, click at end
    let script = format!(
        include_str!("applescript/swipe.applescript.tpl"),
        from_x = from.0,
        from_y = from.1,
        to_x = to.0,
        to_y = to.1,
        duration_sec = duration_sec,
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eyre::bail!("Failed to swipe: {}", stderr.trim());
    }

    Ok(())
}

/// Input text using `AppleScript` keystrokes.
///
/// Sends the given text as keystrokes to the frontmost application.
///
/// # Errors
///
/// Returns an error if the text input fails.
pub async fn text(input: &str) -> eyre::Result<()> {
    // Escape quotes in the input for AppleScript
    let escaped = input.replace('\\', "\\\\").replace('"', "\\\"");

    let script = format!(
        include_str!("applescript/text.applescript.tpl"),
        escaped = escaped,
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eyre::bail!("Failed to input text: {}", stderr.trim());
    }

    // Small delay to let the input register
    smol::Timer::after(Duration::from_millis(50)).await;

    Ok(())
}

/// Capture a screenshot of the entire screen and save to a file.
///
/// Uses the macOS `screencapture` command.
///
/// # Errors
///
/// Returns an error if the screenshot fails.
pub async fn screenshot(output: &Path) -> eyre::Result<()> {
    let output_str = output
        .to_str()
        .ok_or_else(|| eyre!("Invalid output path"))?;

    let result = Command::new("screencapture")
        .arg("-x") // No sound
        .arg(output_str)
        .output()
        .await?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        eyre::bail!("Failed to capture screenshot: {}", stderr.trim());
    }

    Ok(())
}

/// Capture a screenshot of the entire screen and return the raw PNG bytes.
///
/// Uses the macOS `screencapture` command with stdout output.
///
/// # Errors
///
/// Returns an error if the screenshot fails.
pub async fn screenshot_bytes() -> eyre::Result<Vec<u8>> {
    // screencapture can output to stdout with -t png and using - as filename
    let result = Command::new("screencapture")
        .arg("-x") // No sound
        .arg("-t")
        .arg("png")
        .arg("-") // Output to stdout
        .output()
        .await?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        eyre::bail!("Failed to capture screenshot: {}", stderr.trim());
    }

    Ok(result.stdout)
}
