//! `water preview` command implementation.
//!
//! Renders a view function and saves it as a PNG image.

use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use color_eyre::eyre::{Result, bail};

use crate::shell;
use crate::{error, header, success, warn};
use waterui_cli::preview::protocol::{AppError, function_path_symbol_candidates};
use waterui_cli::preview::{PreviewPlatform, launch_preview_session};
use waterui_cli::toolchain::sccache::Sccache;

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

    let symbols = function_path_symbol_candidates(crate_name, &args.function_path);
    header!("Preview: {}", symbols.join(" | "));

    // Detect sccache for compilation caching
    let sccache = Sccache;
    let sccache_path = match sccache.path().await {
        Ok(path) => Some(path),
        Err(_) => {
            warn!(
                "sccache not found. Build efficiency may be reduced. Install with: brew install sccache"
            );
            None
        }
    };

    // Launch preview session (connects to existing app or launches new one)
    let spinner = shell::spinner("Connecting to preview app...");
    let mut session = launch_preview_session(args.platform.into(), sccache_path.clone()).await?;
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

        // Render preview (retry once if preview app connection drops)
        let spinner = shell::spinner("Rendering view...");
        let mut last_err: Option<AppError> = None;
        let mut png_data: Option<Vec<u8>> = None;

        for symbol in &symbols {
            match session
                .client
                .render_with_dylib_source(dylib.id, &dylib.bytes, symbol, width, height)
                .await
            {
                Ok(data) => {
                    png_data = Some(data);
                    break;
                }
                Err(AppError::SymbolNotFound(_)) => {
                    last_err = Some(AppError::SymbolNotFound(symbol.clone()));
                    continue;
                }
                Err(err) => {
                    if should_retry_render_error(&err) {
                        warn!("Preview app connection dropped, relaunching and retrying once...");
                        let _ = session.shutdown().await;
                        session =
                            launch_preview_session(args.platform.into(), sccache_path.clone())
                                .await?;
                        match session
                            .client
                            .render_with_dylib_source(dylib.id, &dylib.bytes, symbol, width, height)
                            .await
                        {
                            Ok(data) => {
                                png_data = Some(data);
                                break;
                            }
                            Err(AppError::SymbolNotFound(_)) => {
                                last_err = Some(AppError::SymbolNotFound(symbol.clone()));
                                continue;
                            }
                            Err(retry_err) => {
                                return Err(color_eyre::eyre::eyre!(
                                    "Preview app error: {retry_err}"
                                ));
                            }
                        }
                    } else {
                        return Err(color_eyre::eyre::eyre!("Preview app error: {err}"));
                    }
                }
            }
        }

        let png_data = match png_data {
            Some(data) => data,
            None => {
                if matches!(last_err, Some(AppError::SymbolNotFound(_))) {
                    bail!(
                        "{}",
                        missing_preview_symbol_message(&args.function_path, &symbols)
                    );
                }
                bail!("Preview app error: no symbol candidates resolved")
            }
        };

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

fn should_retry_render_error(err: &AppError) -> bool {
    match err {
        AppError::RenderFailed(msg) => {
            msg.contains("Failed to receive response")
                || msg.contains("unexpected end of file")
                || msg.contains("Broken pipe")
                || msg.contains("request timed out")
                || msg.contains("transport error")
        }
        _ => false,
    }
}

fn missing_preview_symbol_message(function_path: &str, symbols: &[String]) -> String {
    let expected = symbols
        .iter()
        .map(|s| format!("`{s}`"))
        .collect::<Vec<_>>()
        .join(" or ");

    format!(
        "Preview component not found: `{function_path}`\nExpected export symbol: {expected}\n\
The preview function is likely missing `#[preview]` (or the name is wrong).\n\
Example:\n  #[preview]\n  fn {}() -> impl View {{ ... }}",
        function_path.rsplit("::").next().unwrap_or(function_path)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_retryable_render_errors() {
        assert!(should_retry_render_error(&AppError::RenderFailed(
            "transport error: Failed to receive response".to_string()
        )));
        assert!(should_retry_render_error(&AppError::RenderFailed(
            "request timed out".to_string()
        )));
        assert!(!should_retry_render_error(&AppError::SymbolNotFound(
            "foo".to_string()
        )));
    }

    #[test]
    fn formats_missing_preview_symbol_message() {
        let symbols = vec![
            "waterui_preview_app_dashboard_admin_card_preview".to_string(),
            "waterui_preview_app_card_preview".to_string(),
        ];
        let message = missing_preview_symbol_message("dashboard::admin::card_preview", &symbols);
        assert!(message.contains("dashboard::admin::card_preview"));
        assert!(message.contains("waterui_preview_app_dashboard_admin_card_preview"));
        assert!(message.contains("waterui_preview_app_card_preview"));
        assert!(message.contains("#[preview]"));
        assert!(message.contains("fn card_preview()"));
    }
}
