//! Shared preview request resolution for `water preview` and the MCP
//! `preview` tool.
//!
//! Both entry points accept the same arguments — a `#[preview]` function path
//! or expression target, a frame size, and optional platform/backend/theme
//! overrides — and resolve them through the functions here, so the two can
//! never drift apart.

use clap::ValueEnum;
use eyre::{Result, bail};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::apple::toolchain::AppleSdk;
use crate::preview::protocol::{AppError, DylibId, function_path_to_symbol};
use crate::preview::{
    HydrolysisPreviewSource, HydrolysisPreviewTheme, PreviewPlatform, PreviewSession,
};
use crate::toolchain_checks;

/// Default frame size shared by `water preview --frame` and the MCP `preview`
/// tool's `frame` argument.
pub const DEFAULT_FRAME: &str = "375x667";

/// Target platform for preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CliPreviewBackend {
    /// Apple preview support app.
    Apple,
    /// Android preview support app.
    Android,
    /// Hydrolysis direct renderer.
    Hydrolysis,
}

/// Theme package for Hydrolysis preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
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

/// What a preview render draws: a `#[preview]` function exported from the
/// project crate, or an inline `WaterUI` expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewTarget {
    /// A `#[preview]` function: its crate-relative path and export symbol.
    Function {
        /// Function path as written, e.g. `views::home`.
        function_path: String,
        /// Export symbol the preview machinery looks up.
        symbol: String,
    },
    /// An inline `WaterUI` expression returning `impl View`.
    Expression {
        /// The expression source, e.g. `text("hello")`.
        expression: String,
    },
}

impl PreviewTarget {
    /// Human-readable target name for logs and output file names.
    #[must_use]
    pub fn display_name(&self) -> &str {
        match self {
            Self::Function { symbol, .. } => symbol,
            Self::Expression { expression } => expression,
        }
    }

    /// The [`HydrolysisPreviewSource`] for this target.
    #[must_use]
    pub fn hydrolysis_source(&self) -> HydrolysisPreviewSource<'_> {
        match self {
            Self::Function { symbol, .. } => HydrolysisPreviewSource::Symbol(symbol),
            Self::Expression { expression } => HydrolysisPreviewSource::Expression(expression),
        }
    }
}

/// A fully resolved preview render, ready to hand to the Hydrolysis or
/// support-app execution path.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewRequest {
    /// Resolved target platform.
    pub platform: CliPreviewPlatform,
    /// Resolved rendering backend.
    pub backend: CliPreviewBackend,
    /// Hydrolysis theme package — `Some` iff `backend` is
    /// [`CliPreviewBackend::Hydrolysis`].
    pub hydrolysis_theme: Option<HydrolysisPreviewTheme>,
    /// What to render.
    pub target: PreviewTarget,
    /// Frame width in logical units.
    pub width: f32,
    /// Frame height in logical units.
    pub height: f32,
}

/// Parse frame size from a `WIDTHxHEIGHT` string.
///
/// # Errors
/// Returns an error if the format is wrong or a dimension is not a positive
/// finite number.
pub fn parse_frame(s: &str) -> Result<(f32, f32)> {
    let parts: Vec<&str> = s.split('x').collect();
    if parts.len() != 2 {
        bail!("Invalid frame format: expected WIDTHxHEIGHT (e.g., 375x667)");
    }

    let width: f32 = parts[0]
        .parse()
        .map_err(|_| eyre::eyre!("Invalid frame width"))?;
    let height: f32 = parts[1]
        .parse()
        .map_err(|_| eyre::eyre!("Invalid frame height"))?;

    if !width.is_finite() || width <= 0.0 {
        bail!("Invalid frame width: must be a positive finite number");
    }
    if !height.is_finite() || height <= 0.0 {
        bail!("Invalid frame height: must be a positive finite number");
    }

    Ok((width, height))
}

/// Resolve the preview target: `expr` forces expression mode, and a target
/// that is not a Rust path is treated as an expression either way.
#[must_use]
pub fn resolve_preview_target(
    crate_name: &str,
    target: &str,
    force_expression: bool,
) -> PreviewTarget {
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

/// Resolve the rendering backend for a platform, applying the override when
/// given.
///
/// # Errors
/// Returns an error if the backend does not support the platform.
pub fn resolve_preview_backend(
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

/// Resolve the preview platform, defaulting to this host's native preview
/// platform.
///
/// # Errors
/// Returns an error on hosts with no native preview platform when no override
/// is given.
pub fn resolve_preview_platform(
    platform_override: Option<CliPreviewPlatform>,
) -> Result<CliPreviewPlatform> {
    if let Some(platform) = platform_override {
        return Ok(platform);
    }
    native_preview_platform()
}

// Both lints are host-dependent, so neither `expect` can be fulfilled everywhere:
// on macOS the body is an infallible `const`-compatible `Ok`, while every other host
// bails at runtime with an unsupported-host error.
#[allow(
    clippy::unnecessary_wraps,
    reason = "non-macOS hosts return an explicit unsupported-host error"
)]
#[allow(
    clippy::missing_const_for_fn,
    reason = "non-macOS hosts call the non-const `bail!`"
)]
fn native_preview_platform() -> Result<CliPreviewPlatform> {
    #[cfg(target_os = "macos")]
    {
        Ok(CliPreviewPlatform::Macos)
    }

    #[cfg(not(target_os = "macos"))]
    {
        // `bail!` expands to a `return`, so the trailing semicolon keeps this a
        // statement rather than a macro invocation in expression position.
        bail!(
            "No native preview platform is configured for this host. Pass `--platform` explicitly."
        );
    }
}

/// `water preview test` supports Hydrolysis on macOS only.
///
/// # Errors
/// Returns an error for any other platform.
pub fn ensure_hydrolysis_preview_platform(platform: CliPreviewPlatform) -> Result<()> {
    if platform != CliPreviewPlatform::Macos {
        bail!("`water preview test` supports Hydrolysis on macos only.");
    }
    Ok(())
}

/// Resolve the Hydrolysis theme: required for the Hydrolysis backend,
/// rejected for the others.
///
/// # Errors
/// Returns an error if the theme is missing for Hydrolysis or set for another
/// backend.
pub fn resolve_hydrolysis_preview_theme(
    backend: CliPreviewBackend,
    theme: Option<CliHydrolysisPreviewTheme>,
) -> Result<Option<HydrolysisPreviewTheme>> {
    match (backend, theme) {
        (CliPreviewBackend::Hydrolysis, Some(theme)) => Ok(Some(theme.into())),
        (CliPreviewBackend::Hydrolysis, None) => {
            bail!(
                "Hydrolysis preview requires an explicit theme package. Pass `--theme material3`."
            );
        }
        (_, Some(_)) => {
            bail!("`--theme` is only supported with `--backend hydrolysis`.");
        }
        (_, None) => Ok(None),
    }
}

/// Check the host toolchain required by the resolved backend.
///
/// # Errors
/// Returns an error if a required toolchain component is missing.
pub async fn check_toolchain_for_backend(
    platform: CliPreviewPlatform,
    backend: CliPreviewBackend,
) -> Result<()> {
    let host = crate::toolchain::Host::current();
    match backend {
        CliPreviewBackend::Apple => {
            let sdk = match platform {
                CliPreviewPlatform::Ios => AppleSdk::IosSimulator,
                CliPreviewPlatform::Macos => AppleSdk::Macos,
                CliPreviewPlatform::Android => {
                    bail!("Internal error: Apple preview backend is not supported on android");
                }
            };
            toolchain_checks::check_apple(&host, sdk).await?;
        }
        CliPreviewBackend::Android => {
            if platform != CliPreviewPlatform::Android {
                bail!("Internal error: Android preview backend is not supported on {platform:?}");
            }
            toolchain_checks::check_android_run(&host).await?;
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

/// Render `symbol` through the support-app session, translating a missing
/// export into an actionable `#[preview]` hint.
///
/// # Errors
/// Returns an error if the preview app rejects the render or the transport
/// fails.
pub async fn render_with_symbol(
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
            bail!("{}", missing_preview_symbol_message(function_path, symbol));
        }
        Err(err) => {
            bail!("Preview app error: {err}");
        }
    }
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
        let symbol = "waterui_preview_app_card_preview";
        let message = missing_preview_symbol_message("dashboard::admin::card_preview", symbol);
        assert!(message.contains("dashboard::admin::card_preview"));
        assert!(message.contains("waterui_preview_app_card_preview"));
        assert!(message.contains("#[preview]"));
        assert!(message.contains("fn card_preview()"));
    }
}
