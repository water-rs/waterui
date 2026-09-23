//! `water device` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand};
use color_eyre::eyre::{self, Result};

use crate::{error, line, note, shell, success};
use waterui_cli::{android, apple, capture, gesture};

/// Arguments for the device command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    command: DeviceCommand,
}

/// Device subcommands.
#[derive(Subcommand, Debug)]
pub enum DeviceCommand {
    /// Capture a screenshot from a device.
    Capture(CaptureArgs),

    /// Perform a tap gesture on a device.
    Tap(TapArgs),

    /// Perform a swipe gesture on a device.
    Swipe(SwipeArgs),

    /// Input text on a device.
    Text(TextArgs),

    /// Describe UI elements on the screen (for automation).
    Describe(DescribeArgs),
}

/// Arguments for the capture subcommand.
#[derive(ClapArgs, Debug)]
pub struct CaptureArgs {
    /// Device identifier (UDID for iOS, serial for Android, "local" for macOS).
    /// Mutually exclusive with --pid.
    #[arg(long, conflicts_with = "pid")]
    id: Option<String>,

    /// Process ID for macOS local app window capture.
    /// Mutually exclusive with --id.
    #[arg(long, conflicts_with = "id")]
    pid: Option<i32>,

    /// Capture a specific window by index (0-based). Default: 0 (main window).
    /// Only valid with --pid.
    #[arg(long, requires = "pid")]
    window: Option<usize>,

    /// Capture all windows of the process.
    /// Only valid with --pid.
    #[arg(long, requires = "pid", conflicts_with = "window")]
    all_windows: bool,

    /// Output file path. Defaults to `screenshot_YYYY-MM-DD_HHMMSS.png` in current directory.
    /// For --all-windows, this is ignored; use --output-dir instead.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Output directory for multiple window screenshots (used with --all-windows).
    #[arg(long, requires = "all_windows")]
    output_dir: Option<PathBuf>,
}

/// Arguments for the tap subcommand.
#[derive(ClapArgs, Debug)]
pub struct TapArgs {
    /// Device identifier (UDID for iOS, serial for Android, "local" for macOS).
    #[arg(long)]
    id: String,

    /// X coordinate.
    #[arg(long)]
    x: u32,

    /// Y coordinate.
    #[arg(long)]
    y: u32,

    /// Capture before/after screenshots and output diff info.
    #[arg(long)]
    diff: bool,

    /// Path to save the diff image (only used with --diff).
    #[arg(long)]
    diff_output: Option<PathBuf>,

    /// Delay in milliseconds after gesture before capturing "after" screenshot.
    #[arg(long, default_value = "500")]
    delay: u32,
}

/// Arguments for the swipe subcommand.
#[derive(ClapArgs, Debug)]
pub struct SwipeArgs {
    /// Device identifier (UDID for iOS, serial for Android, "local" for macOS).
    #[arg(long)]
    id: String,

    /// Starting coordinates as "x,y".
    #[arg(long, value_parser = parse_coords)]
    from: (u32, u32),

    /// Ending coordinates as "x,y".
    #[arg(long, value_parser = parse_coords)]
    to: (u32, u32),

    /// Duration of the swipe in milliseconds.
    #[arg(long)]
    duration: Option<u32>,

    /// Capture before/after screenshots and output diff info.
    #[arg(long)]
    diff: bool,

    /// Path to save the diff image (only used with --diff).
    #[arg(long)]
    diff_output: Option<PathBuf>,

    /// Delay in milliseconds after gesture before capturing "after" screenshot.
    #[arg(long, default_value = "500")]
    delay: u32,
}

/// Arguments for the text subcommand.
#[derive(ClapArgs, Debug)]
pub struct TextArgs {
    /// Device identifier (UDID for iOS, serial for Android, "local" for macOS).
    #[arg(long)]
    id: String,

    /// Text to input.
    #[arg(long)]
    input: String,

    /// Capture before/after screenshots and output diff info.
    #[arg(long)]
    diff: bool,

    /// Path to save the diff image (only used with --diff).
    #[arg(long)]
    diff_output: Option<PathBuf>,

    /// Delay in milliseconds after gesture before capturing "after" screenshot.
    #[arg(long, default_value = "500")]
    delay: u32,
}

/// Arguments for the describe subcommand.
#[derive(ClapArgs, Debug)]
pub struct DescribeArgs {
    /// Device identifier (UDID for iOS, serial for Android).
    #[arg(long)]
    id: String,
}

/// Parse coordinate string "x,y" into tuple.
fn parse_coords(s: &str) -> Result<(u32, u32), String> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        return Err("Expected format: x,y (e.g., 100,200)".to_string());
    }
    let x = parts[0]
        .trim()
        .parse::<u32>()
        .map_err(|_| "Invalid X coordinate")?;
    let y = parts[1]
        .trim()
        .parse::<u32>()
        .map_err(|_| "Invalid Y coordinate")?;
    Ok((x, y))
}

/// Run the device command.
pub async fn run(args: Args) -> Result<()> {
    match args.command {
        DeviceCommand::Capture(capture_args) => run_capture(capture_args).await,
        DeviceCommand::Tap(tap_args) => run_tap(tap_args).await,
        DeviceCommand::Swipe(swipe_args) => run_swipe(swipe_args).await,
        DeviceCommand::Text(text_args) => run_text(text_args).await,
        DeviceCommand::Describe(describe_args) => run_describe(describe_args).await,
    }
}

/// Run the capture subcommand.
async fn run_capture(args: CaptureArgs) -> Result<()> {
    // Handle PID-based capture (macOS window capture)
    if let Some(pid) = args.pid {
        return run_capture_by_pid(
            pid,
            args.window,
            args.all_windows,
            args.output,
            args.output_dir,
        )
        .await;
    }

    // Handle device ID-based capture
    let device_id = args.id.as_deref().unwrap_or(gesture::LOCAL_DEVICE_ID);

    // Handle local device for macOS (full screen)
    if device_id == gesture::LOCAL_DEVICE_ID {
        let output = args
            .output
            .unwrap_or_else(capture::generate_screenshot_filename);

        match waterui_cli::apple::local::screenshot(&output).await {
            Ok(()) => {
                success!(
                    "Screenshot saved to {} (from macOS local)",
                    output.display()
                );
                return Ok(());
            }
            Err(e) => {
                error!("Failed to capture screenshot: {e}");
                return Err(e);
            }
        }
    }

    // Verify the device exists
    let platform = match capture::verify_device(device_id).await {
        Ok(p) => p,
        Err(e) => {
            error!("Device not found: {e}");
            return Err(e);
        }
    };

    // Generate output filename if not provided
    let output = args
        .output
        .unwrap_or_else(capture::generate_screenshot_filename);

    // Capture the screenshot
    let platform_name = match platform {
        capture::DevicePlatform::Ios => "iOS simulator",
        capture::DevicePlatform::Android => "Android device",
    };

    match capture::screenshot(device_id, &output).await {
        Ok(()) => {
            success!(
                "Screenshot saved to {} (from {})",
                output.display(),
                platform_name
            );
            Ok(())
        }
        Err(e) => {
            error!("Failed to capture screenshot: {e}");
            Err(e)
        }
    }
}

/// Run capture by PID (macOS window capture).
async fn run_capture_by_pid(
    pid: i32,
    window_index: Option<usize>,
    all_windows: bool,
    output: Option<PathBuf>,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    use waterui_cli::apple::local::{list_windows_by_pid, screenshot_window};

    // Get windows for this PID
    let windows = list_windows_by_pid(pid)?;

    if windows.is_empty() {
        error!("No windows found for PID {pid}");
        eyre::bail!("No windows found for PID {pid}");
    }

    // Filter to only normal windows (layer 0)
    let normal_windows: Vec<_> = windows.iter().filter(|w| w.layer == 0).collect();

    if normal_windows.is_empty() {
        error!(
            "No normal windows found for PID {pid} (found {} auxiliary windows)",
            windows.len()
        );
        eyre::bail!("No normal windows found for PID {pid}");
    }

    if all_windows {
        // Capture all windows
        let dir = output_dir.unwrap_or_else(|| PathBuf::from("."));
        smol::fs::create_dir_all(&dir).await?;

        for (i, window) in normal_windows.iter().enumerate() {
            let filename = if window.name.is_empty() {
                format!("window_{i}.png")
            } else {
                // Sanitize window name for filename
                let safe_name: String = window
                    .name
                    .chars()
                    .map(|c| {
                        if c.is_alphanumeric() || c == '-' || c == '_' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                format!("window_{i}_{safe_name}.png")
            };
            let path = dir.join(&filename);

            match screenshot_window(window.window_id, &path).await {
                Ok(()) => {
                    success!("Window {i} \"{}\" saved to {}", window.name, path.display());
                }
                Err(e) => {
                    error!("Failed to capture window {i}: {e}");
                }
            }
        }

        note!("Captured {} windows for PID {pid}", normal_windows.len());
        Ok(())
    } else {
        // Capture single window
        let index = window_index.unwrap_or(0);

        if index >= normal_windows.len() {
            error!(
                "Window index {index} out of range (found {} windows)",
                normal_windows.len()
            );
            eyre::bail!(
                "Window index {index} out of range (found {} windows)",
                normal_windows.len()
            );
        }

        let window = &normal_windows[index];
        let output_path = output.unwrap_or_else(capture::generate_screenshot_filename);

        match screenshot_window(window.window_id, &output_path).await {
            Ok(()) => {
                success!(
                    "Screenshot saved to {} (window \"{}\" from PID {pid})",
                    output_path.display(),
                    window.name
                );
                Ok(())
            }
            Err(e) => {
                error!("Failed to capture screenshot: {e}");
                Err(e)
            }
        }
    }
}

/// Build gesture options from args.
const fn build_gesture_options(
    diff: bool,
    diff_output: Option<PathBuf>,
    delay: u32,
) -> gesture::GestureOptions {
    gesture::GestureOptions {
        diff,
        diff_output,
        delay_ms: Some(delay),
    }
}

/// Print diff result if present.
fn print_diff_result(result: &gesture::GestureResult, diff_output: Option<&std::path::Path>) {
    if let Some(diff) = &result.diff {
        if let Some(path) = diff_output {
            success!("Diff image saved to {}", path.display());
        }
        note!("Diff result:\n{diff}");
    }
}

/// Run the tap subcommand.
async fn run_tap(args: TapArgs) -> Result<()> {
    let device_id = &args.id;

    // Verify device exists
    gesture::verify_device(device_id).await?;

    let options = build_gesture_options(args.diff, args.diff_output.clone(), args.delay);

    match gesture::tap(device_id, args.x, args.y, &options).await {
        Ok(result) => {
            success!("Tap at ({}, {})", args.x, args.y);
            print_diff_result(&result, args.diff_output.as_deref());
            Ok(())
        }
        Err(e) => {
            error!("Failed to tap: {e}");
            Err(e)
        }
    }
}

/// Run the swipe subcommand.
async fn run_swipe(args: SwipeArgs) -> Result<()> {
    let device_id = &args.id;

    // Verify device exists
    gesture::verify_device(device_id).await?;

    let options = build_gesture_options(args.diff, args.diff_output.clone(), args.delay);

    match gesture::swipe(device_id, args.from, args.to, args.duration, &options).await {
        Ok(result) => {
            success!(
                "Swipe from ({}, {}) to ({}, {})",
                args.from.0,
                args.from.1,
                args.to.0,
                args.to.1
            );
            print_diff_result(&result, args.diff_output.as_deref());
            Ok(())
        }
        Err(e) => {
            error!("Failed to swipe: {e}");
            Err(e)
        }
    }
}

/// Run the text subcommand.
async fn run_text(args: TextArgs) -> Result<()> {
    let device_id = &args.id;

    // Verify device exists
    gesture::verify_device(device_id).await?;

    let options = build_gesture_options(args.diff, args.diff_output.clone(), args.delay);

    match gesture::text(device_id, &args.input, &options).await {
        Ok(result) => {
            success!("Text input: \"{}\"", args.input);
            print_diff_result(&result, args.diff_output.as_deref());
            Ok(())
        }
        Err(e) => {
            error!("Failed to input text: {e}");
            Err(e)
        }
    }
}

/// Run the describe subcommand.
async fn run_describe(args: DescribeArgs) -> Result<()> {
    let device_id = &args.id;

    // Local macOS device is not supported
    if device_id == gesture::LOCAL_DEVICE_ID {
        eyre::bail!("Describe is not supported for local macOS device");
    }

    // Get platform and call appropriate describe function
    let json = match capture::detect_platform(device_id) {
        capture::DevicePlatform::Ios => apple::device::describe(device_id).await?,
        capture::DevicePlatform::Android => android::device::describe(device_id).await?,
    };

    if shell::get().is_json() {
        // JSON mode: output raw JSON
        shell::json_raw(&json);
    } else {
        // Readable mode: format as table
        print_ui_elements_readable(&json)?;
    }

    Ok(())
}

/// Print UI elements in human-readable format.
fn print_ui_elements_readable(json: &str) -> Result<()> {
    let elements: Vec<serde_json::Value> = serde_json::from_str(json)?;

    line!("UI Elements ({} found):", elements.len());
    line!("{}", "-".repeat(80));

    for (i, elem) in elements.iter().enumerate() {
        let label = elem.get("AXLabel").and_then(|v| v.as_str()).unwrap_or("-");
        let elem_type = elem.get("type").and_then(|v| v.as_str()).unwrap_or("-");
        let value = elem.get("AXValue").and_then(|v| v.as_str()).unwrap_or("");

        // Get frame info
        let frame = elem.get("frame");
        let (x, y, w, h) = frame.map_or((0.0, 0.0, 0.0, 0.0), |frame| {
            (
                frame
                    .get("x")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0),
                frame
                    .get("y")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0),
                frame
                    .get("width")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0),
                frame
                    .get("height")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0),
            )
        });

        // Calculate center point for tapping
        let center_x = x + w / 2.0;
        let center_y = y + h / 2.0;

        // Only show elements with a label or value
        if label != "-" || !value.is_empty() {
            let display_value = if value.is_empty() { label } else { value };
            let label_suffix = if value.is_empty() || label == "-" {
                String::new()
            } else {
                format!(" ({label})")
            };
            line!(
                "[{}] {} \"{}\"{}",
                i,
                elem_type,
                display_value,
                label_suffix
            );
            line!(
                "    tap: --x {center_x:.0} --y {center_y:.0}  (frame: {x:.0},{y:.0} {w:.0}x{h:.0})"
            );
        }
    }

    Ok(())
}
