//! `water preview` command implementation.
//!
//! Renders a view function and saves it as a PNG image.

use std::path::PathBuf;
use std::time::Instant;

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

    /// Render the same preview multiple times (useful for benchmarking).
    #[arg(long, default_value_t = 1)]
    repeat: u32,

    /// Print timing information to stderr.
    #[arg(long)]
    timing: bool,

    /// Project directory path (defaults to current directory).
    #[arg(long, default_value = ".")]
    path: PathBuf,
}

/// Run the preview command.
///
/// # Errors
/// Returns an error if preview fails.
pub async fn run(args: Args) -> Result<()> {
    // Parse frame size
    let (width, height) = parse_frame(&args.frame)?;

    // Canonicalize project path
    let project_path = args.path.canonicalize()?;

    // Get crate name from Cargo.toml
    let cargo_toml = project_path.join("Cargo.toml");
    let cargo_content = smol::fs::read_to_string(&cargo_toml).await?;
    let cargo: toml::Table = cargo_content.parse()?;
    let crate_name = cargo
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .ok_or_else(|| color_eyre::eyre::eyre!("Could not find package name in Cargo.toml"))?;

    let symbol = function_path_to_symbol(crate_name, &args.function_path);
    header!("Preview: {}", symbol);

    if args.repeat == 0 {
        bail!("--repeat must be >= 1");
    }

    // Launch preview session (connects to existing app or launches new one)
    let connect_start = Instant::now();
    let spinner = shell::spinner("Connecting to preview app...");
    let mut session = launch_preview_session(args.platform.into()).await?;
    if let Some(s) = spinner {
        s.finish_and_clear();
    }
    let connect_elapsed = connect_start.elapsed();

    let result = async {
        // Build dylib
        let build_start = Instant::now();
        let spinner = shell::spinner("Building project...");
        let dylib = session.build_dylib(&project_path).await?;
        if let Some(s) = spinner {
            s.finish_and_clear();
        }
        let build_elapsed = build_start.elapsed();

        let mut render_times = Vec::with_capacity(args.repeat as usize);

        for i in 0..args.repeat {
            // Render preview
            let spinner = shell::spinner("Rendering view...");
            let render_start = Instant::now();
            let png_data = session.render(&dylib, &symbol, width, height).await?;
            render_times.push(render_start.elapsed());

            if let Some(s) = spinner {
                s.finish_and_clear();
            }

            // If repeating, write each output as <name>-<n>.<ext>
            let output_path = if args.repeat == 1 {
                args.output.clone()
            } else {
                numbered_output_path(&args.output, i + 1)
            };

            if png_data.is_empty() {
                error!("Preview returned empty PNG data");
                bail!("Preview returned empty PNG data");
            }
            smol::fs::write(&output_path, &png_data).await?;
            success!("Preview saved to {}", output_path.display());
        }

        if args.timing {
            eprintln!("[timing] connect: {:?}", connect_elapsed);
            eprintln!("[timing] build:   {:?}", build_elapsed);
            for (i, t) in render_times.iter().enumerate() {
                eprintln!("[timing] render#{:02}: {:?}", i + 1, t);
            }
        }
        Ok(())
    }
    .await;

    // Keep preview app running for reuse by future preview commands.
    session.detach();

    result
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

fn numbered_output_path(base: &PathBuf, n: u32) -> PathBuf {
    let parent = base.parent().unwrap_or_else(|| std::path::Path::new("."));
    let stem = base
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("preview");
    let ext = base.extension().and_then(|e| e.to_str());

    let filename = match ext {
        Some(ext) if !ext.is_empty() => format!("{stem}-{n}.{ext}"),
        _ => format!("{stem}-{n}"),
    };
    parent.join(filename)
}
