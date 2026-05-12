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
    HydrolysisPreviewSource, HydrolysisPreviewTheme, PreviewPlatform, PreviewSession,
    launch_preview_session, render_preview_with_hydrolysis,
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
            CliPreviewPlatform::Ios => Self::IosSimulator,
            CliPreviewPlatform::Macos => Self::Macos,
            CliPreviewPlatform::Android => Self::Android,
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

/// Theme package for Hydrolysis preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliHydrolysisPreviewTheme {
    /// Material Design 3 theme package.
    Material3,
}

impl From<CliHydrolysisPreviewTheme> for HydrolysisPreviewTheme {
    fn from(value: CliHydrolysisPreviewTheme) -> Self {
        match value {
            CliHydrolysisPreviewTheme::Material3 => Self::Material3,
        }
    }
}

/// Arguments for the preview command.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Preview target: a `#[preview]` function path or a WaterUI expression.
    target: String,

    /// Treat the target as a WaterUI expression returning `impl View`.
    #[arg(long)]
    expr: bool,

    /// Target platform.
    #[arg(short, long, value_enum)]
    platform: CliPreviewPlatform,

    /// Rendering backend.
    #[arg(long, value_enum)]
    backend: Option<CliPreviewBackend>,

    /// Theme package for Hydrolysis preview.
    #[arg(long, value_enum)]
    theme: Option<CliHydrolysisPreviewTheme>,

    /// Frame size `WIDTHxHEIGHT` (default: `375x667`).
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

    let backend = resolve_preview_backend(args.platform, args.backend)?;
    let hydrolysis_theme = resolve_hydrolysis_preview_theme(backend, args.theme)?;
    let preview_target = resolve_preview_target(crate_name, &args.target, args.expr);
    header!("Preview: {}", preview_target.display_name());

    check_toolchain_for_backend(args.platform, backend).await?;

    // Detect sccache for compilation caching
    let sccache = Sccache;
    let sccache_path = sccache.path().await.map_or_else(
        |_| {
            warn!(
                "sccache not found. Build efficiency may be reduced. Install with: {}",
                sccache_install_hint()
            );
            None
        },
        Some,
    );

    if backend == CliPreviewBackend::Hydrolysis {
        let spinner = shell::spinner("Building and rendering with hydrolysis...");
        render_preview_with_hydrolysis(
            &project_path,
            preview_target.hydrolysis_source(),
            hydrolysis_theme.expect("hydrolysis preview theme must be resolved"),
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

    let PreviewTarget::Function {
        function_path,
        symbol,
    } = &preview_target
    else {
        bail!("Expression preview is currently supported only with `--backend hydrolysis`.");
    };

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
            function_path,
            symbol,
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
            session.shutdown().await?;
            Err(err)
        }
    }
}

#[derive(Debug)]
enum PreviewTarget {
    Function {
        function_path: String,
        symbol: String,
    },
    Expression {
        expression: String,
    },
}

impl PreviewTarget {
    fn display_name(&self) -> &str {
        match self {
            Self::Function { symbol, .. } => symbol,
            Self::Expression { expression } => expression,
        }
    }

    fn hydrolysis_source(&self) -> HydrolysisPreviewSource<'_> {
        match self {
            Self::Function { symbol, .. } => HydrolysisPreviewSource::Symbol(symbol),
            Self::Expression { expression } => HydrolysisPreviewSource::Expression(expression),
        }
    }
}

fn resolve_preview_target(crate_name: &str, target: &str, force_expression: bool) -> PreviewTarget {
    if force_expression || !is_function_path(target) {
        return PreviewTarget::Expression {
            expression: target.to_string(),
        };
    }

    PreviewTarget::Function {
        function_path: target.to_string(),
        symbol: function_path_to_symbol(crate_name, target),
    }
}

fn is_function_path(target: &str) -> bool {
    let mut segments = target.split("::").peekable();
    if segments.peek().is_none() {
        return false;
    }

    segments.all(is_rust_ident)
}

fn is_rust_ident(segment: &str) -> bool {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
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
        (
            CliPreviewPlatform::Ios | CliPreviewPlatform::Macos,
            CliPreviewBackend::Apple
        ) | (CliPreviewPlatform::Macos, CliPreviewBackend::Hydrolysis)
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

fn resolve_hydrolysis_preview_theme(
    backend: CliPreviewBackend,
    theme: Option<CliHydrolysisPreviewTheme>,
) -> Result<Option<HydrolysisPreviewTheme>> {
    match (backend, theme) {
        (CliPreviewBackend::Hydrolysis, Some(theme)) => Ok(Some(theme.into())),
        (CliPreviewBackend::Hydrolysis, None) => {
            bail!(
                "Hydrolysis preview requires an explicit theme package. Pass `--theme material3`."
            )
        }
        (_, Some(_)) => bail!("`--theme` is only supported with `--backend hydrolysis`."),
        (_, None) => Ok(None),
    }
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
    let prefer_local_path = session.platform == PreviewPlatform::Macos;
    match session
        .client
        .render_with_dylib_file(
            dylib_id,
            dylib_path,
            symbol,
            width,
            height,
            prefer_local_path,
        )
        .await
    {
        Ok(data) => Ok(data),
        Err(AppError::SymbolNotFound(_)) => {
            bail!("{}", missing_preview_symbol_message(function_path, symbol))
        }
        Err(err) => bail!("Preview app error: {err}"),
    }
}

/// Parse frame size from `WIDTHxHEIGHT` string.
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

    #[test]
    fn resolves_plain_path_as_preview_function() {
        let target = resolve_preview_target("my-crate", "dashboard::card", false);
        let PreviewTarget::Function {
            function_path,
            symbol,
        } = target
        else {
            panic!("expected function target");
        };
        assert_eq!(function_path, "dashboard::card");
        assert_eq!(symbol, "waterui_preview_my_crate_dashboard_card");
    }

    #[test]
    fn resolves_expression_syntax_as_expression_preview() {
        let target = resolve_preview_target("my-crate", "button(\"Save\")", false);
        let PreviewTarget::Expression { expression } = target else {
            panic!("expected expression target");
        };
        assert_eq!(expression, "button(\"Save\")");
    }

    #[test]
    fn expr_flag_forces_identifier_as_expression_preview() {
        let target = resolve_preview_target("my-crate", "main_view", true);
        let PreviewTarget::Expression { expression } = target else {
            panic!("expected expression target");
        };
        assert_eq!(expression, "main_view");
    }

    #[test]
    fn hydrolysis_preview_requires_explicit_theme() {
        let result = resolve_hydrolysis_preview_theme(CliPreviewBackend::Hydrolysis, None);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_theme_for_non_hydrolysis_preview() {
        let result = resolve_hydrolysis_preview_theme(
            CliPreviewBackend::Apple,
            Some(CliHydrolysisPreviewTheme::Material3),
        );
        assert!(result.is_err());
    }
}
