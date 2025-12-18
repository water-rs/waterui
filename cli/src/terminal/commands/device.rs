//! `water device` command implementation.

use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand};
use color_eyre::eyre::Result;

use crate::{error, success};
use waterui_cli::capture;

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
}

/// Arguments for the capture subcommand.
#[derive(ClapArgs, Debug)]
pub struct CaptureArgs {
    /// Device identifier (UDID for iOS, serial for Android).
    #[arg(long)]
    id: String,

    /// Output file path. Defaults to `screenshot_YYYY-MM-DD_HHMMSS.png` in current directory.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

/// Run the device command.
pub async fn run(args: Args) -> Result<()> {
    match args.command {
        DeviceCommand::Capture(capture_args) => run_capture(capture_args).await,
    }
}

/// Run the capture subcommand.
async fn run_capture(args: CaptureArgs) -> Result<()> {
    let device_id = &args.id;

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
