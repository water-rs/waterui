//! macOS local device gestures and screenshot support.
//!
//! Provides gesture automation for the current macOS machine using AppleScript
//! and the `screencapture` command for screenshots.

use std::path::Path;
use std::time::Duration;

use color_eyre::eyre::{self, eyre};
use smol::process::Command;

/// Perform a tap (click) gesture at the specified screen coordinates.
///
/// Uses AppleScript to click at the given absolute screen position.
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
        r#"
        tell application "System Events"
            click at {{{x}, {y}}}
        end tell
        "#
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
/// Uses AppleScript to simulate a drag from one point to another.
/// Note: AppleScript's built-in drag is limited; this uses a click-based approximation.
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
    let duration_sec = duration_ms.unwrap_or(300) as f64 / 1000.0;

    // AppleScript doesn't have native drag support
    // We simulate with click at start, delay, click at end
    let script = format!(
        r#"
        tell application "System Events"
            click at {{{}, {}}}
            delay {}
            click at {{{}, {}}}
        end tell
        "#,
        from.0, from.1, duration_sec, to.0, to.1
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

/// Input text using AppleScript keystrokes.
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
        r#"
        tell application "System Events"
            keystroke "{escaped}"
        end tell
        "#
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
