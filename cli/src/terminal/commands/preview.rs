//! `water preview` command implementation.
//!
//! Renders a view function and saves it as a PNG image.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};

use crate::shell;
use crate::{error, header, success};
use waterui_cli::preview::protocol::function_path_to_symbol;
use waterui_cli::preview::{PreviewPlatform, launch_preview_session};

/// Target platform for preview.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliPreviewPlatform {
    /// iOS Simulator.
    Ios,
    /// macOS.
    Macos,
    /// Android Emulator.
    Android,
}

impl From<CliPreviewPlatform> for PreviewPlatform {
    fn from(p: CliPreviewPlatform) -> Self {
        match p {
            CliPreviewPlatform::Ios => PreviewPlatform::IosSimulator,
            CliPreviewPlatform::Macos => PreviewPlatform::Macos,
            CliPreviewPlatform::Android => PreviewPlatform::Android,
        }
    }
}

/// Arguments for the preview command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Function path (e.g., `dashboard::admin::card`).
    function_path: String,

    /// Target platform.
    #[arg(short, long, value_enum)]
    platform: CliPreviewPlatform,

    /// Frame size "WIDTHxHEIGHT" (default: 375x667).
    #[arg(short, long, default_value = "375x667")]
    frame: String,

    /// Output file (default: preview.png).
    #[arg(short, long, default_value = "preview.png")]
    output: PathBuf,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

/// Run the preview command.
///
/// # Errors
/// Returns an error if preview fails.
pub async fn run(args: Args) -> Result<()> {
    let symbol = function_path_to_symbol(&args.function_path);
    header!("Preview: {}", symbol);

    // Parse frame size
    let (width, height) = parse_frame(&args.frame)?;

    // Canonicalize project path
    let project_path = args.path.canonicalize()?;

    // Launch preview session (connects to existing app or launches new one)
    let spinner = shell::spinner("Connecting to preview app...");
    let mut session = launch_preview_session(args.platform.into()).await?;
    if let Some(s) = spinner {
        s.finish_and_clear();
    }

    // Build dylib
    let spinner = shell::spinner("Building project...");
    let dylib_data = session.build_dylib(&project_path).await?;
    if let Some(s) = spinner {
        s.finish_and_clear();
    }

    // Render preview
    let spinner = shell::spinner("Rendering view...");
    let png_data = session.render(&dylib_data, &symbol, width, height)?;
    if let Some(s) = spinner {
        s.finish_and_clear();
    }

    // Save output
    if png_data.is_empty() {
        error!("Preview returned empty PNG data");
        bail!("Preview returned empty PNG data");
    }

    smol::fs::write(&args.output, &png_data).await?;
    success!("Preview saved to {}", args.output.display());
    Ok(())
}

/// Parse frame size from "WIDTHxHEIGHT" string.
fn parse_frame(s: &str) -> Result<(f32, f32)> {
    let parts: Vec<&str> = s.split('x').collect();
    if parts.len() != 2 {
        bail!("Invalid frame format: expected WIDTHxHEIGHT (e.g., 375x667)");
    }

    let width: f32 = parts[0]
        .parse()
        .map_err(|_| color_eyre::eyre::eyre!("Invalid frame width"))?;
    let height: f32 = parts[1]
        .parse()
        .map_err(|_| color_eyre::eyre::eyre!("Invalid frame height"))?;

    Ok((width, height))
}
