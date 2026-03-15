//! `water preview` command implementation.
//!
//! Renders a view function and saves it as a PNG image.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};

use crate::shell;
use crate::toolchain_checks;
use crate::{error, header, success, warn};
use waterui_cli::preview::protocol::{AppError, DylibId, function_path_to_symbol};
use waterui_cli::preview::{
    PreviewPlatform, PreviewSession, launch_preview_session, render_preview_with_hydrolysis,
};
use waterui_cli::toolchain::sccache::Sccache;
use waterui_cli::utils::sccache_install_hint;

/// Target platform for preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
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

/// Rendering backend for preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliPreviewBackend {
    /// Apple preview support app.
    Apple,
    /// Android preview support app.
    Android,
    /// Hydrolysis direct renderer.
    Hydrolysis,
}

/// Arguments for the preview command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Function path (e.g., `dashboard::admin::card`).
    function_path: String,

    /// Target platform.
    #[arg(short, long, value_enum)]
    platform: CliPreviewPlatform,

    /// Rendering backend.
    #[arg(long, value_enum)]
    backend: Option<CliPreviewBackend>,

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
    // Parse frame size
    let (width, height) = parse_frame(&args.frame)?;

    // Canonicalize project path
    let project_path = crate::project_path::canonicalize(&args.path)?;

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
    let backend = resolve_preview_backend(args.platform, args.backend)?;
    header!("Preview: {symbol}");

    check_toolchain_for_backend(args.platform, backend).await?;

    // Detect sccache for compilation caching
    let sccache = Sccache;
    let sccache_path = match sccache.path().await {
        Ok(path) => Some(path),
        Err(_) => {
            warn!(
                "sccache not found. Build efficiency may be reduced. Install with: {}",
                sccache_install_hint()
            );
            None
        }
    };

    if backend == CliPreviewBackend::Hydrolysis {
        let spinner = shell::spinner("Building and rendering with hydrolysis...");
        render_preview_with_hydrolysis(
            &project_path,
            &symbol,
            width,
            height,
            sccache_path,
            &args.output,
        )
        .await?;
        if let Some(s) = spinner {
            s.finish_and_clear();
        }
        success!("Preview saved to {}", args.output.display());
        return Ok(());
    }

    // Launch preview session (connects to existing app or launches new one)
    let spinner = shell::spinner("Connecting to preview app...");
    let platform: PreviewPlatform = args.platform.into();
    let mut session = launch_preview_session(&project_path, platform, sccache_path.clone()).await?;
    if let Some(s) = spinner {
        s.finish_and_clear();
    }

    let result = async {
        // Build dylib
        let spinner = shell::spinner("Building project...");
        let dylib = session.build_dylib(&project_path).await?;
        if let Some(s) = spinner {
            s.finish_and_clear();
        }

        let spinner = shell::spinner("Rendering view...");
        let png_data = render_with_symbol(
            &mut session,
            &args.function_path,
            &symbol,
            dylib.id,
            &dylib.path,
            width,
            height,
        )
        .await?;
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
    .await;

    match result {
        Ok(()) => {
            // Keep preview app running for reuse by future preview commands.
            session.detach();
            Ok(())
        }
        Err(err) => {
            // On failure, terminate the preview app to avoid reusing a broken process.
            let _ = session.shutdown().await;
            Err(err)
        }
    }
}

fn resolve_preview_backend(
    platform: CliPreviewPlatform,
    backend_override: Option<CliPreviewBackend>,
) -> Result<CliPreviewBackend> {
    let default_backend = match platform {
        CliPreviewPlatform::Ios | CliPreviewPlatform::Macos => CliPreviewBackend::Apple,
        CliPreviewPlatform::Android => CliPreviewBackend::Android,
    };

    let backend = backend_override.unwrap_or(default_backend);
    let supported = matches!(
        (platform, backend),
        (CliPreviewPlatform::Ios, CliPreviewBackend::Apple)
            | (CliPreviewPlatform::Macos, CliPreviewBackend::Apple)
            | (CliPreviewPlatform::Macos, CliPreviewBackend::Hydrolysis)
            | (CliPreviewPlatform::Android, CliPreviewBackend::Android)
    );
    if !supported {
        bail!(
            "Preview backend {:?} does not support platform {:?}. Valid combinations: ios/apple, macos/apple, macos/hydrolysis, android/android",
            backend,
            platform
        );
    }
    Ok(backend)
}

async fn check_toolchain_for_backend(
    platform: CliPreviewPlatform,
    backend: CliPreviewBackend,
) -> Result<()> {
    match backend {
        CliPreviewBackend::Apple => {
            let sdk = match platform {
                CliPreviewPlatform::Ios => waterui_cli::apple::toolchain::AppleSdk::IosSimulator,
                CliPreviewPlatform::Macos => waterui_cli::apple::toolchain::AppleSdk::Macos,
                CliPreviewPlatform::Android => {
                    bail!("Internal error: Apple preview backend is not supported on android")
                }
            };
            toolchain_checks::check_apple(sdk).await?;
        }
        CliPreviewBackend::Android => {
            if platform != CliPreviewPlatform::Android {
                bail!("Internal error: Android preview backend is not supported on {platform:?}");
            }
            toolchain_checks::check_android_run().await?;
        }
        CliPreviewBackend::Hydrolysis => {
            if platform != CliPreviewPlatform::Macos {
                bail!(
                    "Internal error: Hydrolysis preview backend is not supported on {platform:?}"
                );
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn render_with_symbol(
    session: &mut PreviewSession,
    function_path: &str,
    symbol: &str,
    dylib_id: DylibId,
    dylib_path: &std::path::Path,
    width: f32,
    height: f32,
) -> Result<Vec<u8>> {
    match session
        .client
        .render_with_dylib_file(dylib_id, dylib_path, symbol, width, height)
        .await
    {
        Ok(data) => Ok(data),
        Err(AppError::SymbolNotFound(_)) => {
            bail!("{}", missing_preview_symbol_message(function_path, symbol))
        }
        Err(err) => bail!("Preview app error: {err}"),
    }
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

    if !width.is_finite() || width <= 0.0 {
        bail!("Invalid frame width: must be a positive finite number");
    }
    if !height.is_finite() || height <= 0.0 {
        bail!("Invalid frame height: must be a positive finite number");
    }

    Ok((width, height))
}

fn missing_preview_symbol_message(function_path: &str, symbol: &str) -> String {
    format!(
        "Preview component not found: `{function_path}`\nExpected export symbol: `{symbol}`\n\
The preview function is likely missing `#[preview]` (or the name is wrong).\n\
Example:\n  #[preview]\n  fn {}() -> impl View {{ ... }}",
        function_path.rsplit("::").next().unwrap_or(function_path)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_missing_preview_symbol_message() {
        let symbol = "waterui_preview_app_dashboard_admin_card_preview";
        let message = missing_preview_symbol_message("dashboard::admin::card_preview", symbol);
        assert!(message.contains("dashboard::admin::card_preview"));
        assert!(message.contains("waterui_preview_app_dashboard_admin_card_preview"));
        assert!(message.contains("#[preview]"));
        assert!(message.contains("fn card_preview()"));
    }

    #[test]
    fn rejects_non_positive_frame_values() {
        assert!(parse_frame("0x100").is_err());
        assert!(parse_frame("-1x100").is_err());
        assert!(parse_frame("100x0").is_err());
        assert!(parse_frame("100x-1").is_err());
    }

    #[test]
    fn rejects_non_finite_frame_values() {
        assert!(parse_frame("NaNx100").is_err());
        assert!(parse_frame("100xinf").is_err());
    }
}
