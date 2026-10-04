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
use crate::platform::TargetPlatform;
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
    /// Linux (Hydrolysis).
    Linux,
    /// Windows (Hydrolysis).
    Windows,
}

impl CliPreviewPlatform {
    /// The desktop OS this platform renders on, if it is one. iOS and
    /// Android previews are device targets and return `None`.
    const fn desktop_os(self) -> Option<&'static str> {
        match self {
            Self::Macos => Some("macos"),
            Self::Linux => Some("linux"),
            Self::Windows => Some("windows"),
            Self::Ios | Self::Android => None,
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

/// The preview execution path `resolve_preview_backend` resolved, carrying
/// the target each path builds for or serves.
///
/// A Hydrolysis render compiles a managed backend binary for a desktop
/// [`TargetPlatform`]; a support-app render talks the preview protocol to a
/// [`PreviewPlatform`]. Carrying the target on the variant keeps call sites
/// from re-deriving it from the CLI platform label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedPreviewBackend {
    /// Hydrolysis render for the given desktop target.
    Hydrolysis(TargetPlatform),
    /// Preview support-app render for the given protocol platform.
    SupportApp(PreviewPlatform),
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
    /// Resolved rendering backend and the target it builds for or serves.
    pub backend: ResolvedPreviewBackend,
    /// Hydrolysis theme package — `Some` iff `backend` is
    /// [`ResolvedPreviewBackend::Hydrolysis`].
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
/// The result carries the target the resolved path needs: the desktop
/// [`TargetPlatform`] Hydrolysis builds for, or the [`PreviewPlatform`] a
/// support app serves.
///
/// # Errors
/// Returns an error if the backend does not support the platform.
pub fn resolve_preview_backend(
    platform: CliPreviewPlatform,
    backend_override: Option<CliPreviewBackend>,
) -> Result<ResolvedPreviewBackend> {
    let default_backend = match platform {
        CliPreviewPlatform::Ios | CliPreviewPlatform::Macos => CliPreviewBackend::Apple,
        CliPreviewPlatform::Android => CliPreviewBackend::Android,
        CliPreviewPlatform::Linux | CliPreviewPlatform::Windows => CliPreviewBackend::Hydrolysis,
    };

    Ok(
        match (platform, backend_override.unwrap_or(default_backend)) {
            (CliPreviewPlatform::Ios, CliPreviewBackend::Apple) => {
                ResolvedPreviewBackend::SupportApp(PreviewPlatform::IosSimulator)
            }
            (CliPreviewPlatform::Macos, CliPreviewBackend::Apple) => {
                ResolvedPreviewBackend::SupportApp(PreviewPlatform::Macos)
            }
            (CliPreviewPlatform::Macos, CliPreviewBackend::Hydrolysis) => {
                ResolvedPreviewBackend::Hydrolysis(TargetPlatform::MacOS)
            }
            (CliPreviewPlatform::Linux, CliPreviewBackend::Hydrolysis) => {
                ResolvedPreviewBackend::Hydrolysis(TargetPlatform::Linux)
            }
            (CliPreviewPlatform::Windows, CliPreviewBackend::Hydrolysis) => {
                ResolvedPreviewBackend::Hydrolysis(TargetPlatform::Windows)
            }
            (CliPreviewPlatform::Android, CliPreviewBackend::Android) => {
                ResolvedPreviewBackend::SupportApp(PreviewPlatform::Android)
            }
            (platform, backend) => {
                bail!(
                    "Preview backend {backend:?} does not support platform {platform:?}. Valid combinations: ios/apple, macos/apple, macos/hydrolysis, linux/hydrolysis, windows/hydrolysis, android/android"
                );
            }
        },
    )
}

/// Resolve the preview platform, defaulting to this host's native preview
/// platform.
///
/// # Errors
/// Returns an error on hosts with no native preview platform when no override
/// is given, or when the override names a desktop OS other than the host's.
pub fn resolve_preview_platform(
    platform_override: Option<CliPreviewPlatform>,
) -> Result<CliPreviewPlatform> {
    let platform = platform_override.map_or_else(native_preview_platform, Ok)?;
    crate::platform::ensure_desktop_platform_is_host(platform.desktop_os(), std::env::consts::OS)?;
    Ok(platform)
}

fn native_preview_platform() -> Result<CliPreviewPlatform> {
    native_preview_platform_for_os(std::env::consts::OS).ok_or_else(|| {
        eyre::eyre!(
            "No native preview platform is configured for this host. Pass `--platform` explicitly."
        )
    })
}

/// The preview platform a host OS renders natively: `macos` through the Apple
/// support app, `linux` and `windows` through the Hydrolysis backend — the
/// same renderer `water run` uses on those hosts.
fn native_preview_platform_for_os(os: &str) -> Option<CliPreviewPlatform> {
    match os {
        "macos" => Some(CliPreviewPlatform::Macos),
        "linux" => Some(CliPreviewPlatform::Linux),
        "windows" => Some(CliPreviewPlatform::Windows),
        _ => None,
    }
}

/// `water preview test` renders through Hydrolysis only — resolve the
/// desktop target it builds for.
///
/// # Errors
/// Returns an error when `platform` does not support the Hydrolysis backend.
pub fn resolve_hydrolysis_test_platform(platform: CliPreviewPlatform) -> Result<TargetPlatform> {
    match resolve_preview_backend(platform, Some(CliPreviewBackend::Hydrolysis))? {
        ResolvedPreviewBackend::Hydrolysis(target) => Ok(target),
        ResolvedPreviewBackend::SupportApp(_) => {
            unreachable!("a forced Hydrolysis backend resolves to Hydrolysis")
        }
    }
}

/// Resolve the Hydrolysis theme: defaulted for the Hydrolysis backend,
/// rejected for the others.
///
/// Hydrolysis is the native preview platform on Linux and Windows, so its
/// theme cannot be a required flag there — `material3` is the only theme
/// package today and is the default until a second one exists.
///
/// # Errors
/// Returns an error if the theme is set for a non-Hydrolysis backend.
pub fn resolve_hydrolysis_preview_theme(
    backend: ResolvedPreviewBackend,
    theme: Option<CliHydrolysisPreviewTheme>,
) -> Result<Option<HydrolysisPreviewTheme>> {
    match (backend, theme) {
        (ResolvedPreviewBackend::Hydrolysis(_), theme) => Ok(Some(
            theme.unwrap_or(CliHydrolysisPreviewTheme::Material3).into(),
        )),
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
pub async fn check_toolchain_for_backend(backend: ResolvedPreviewBackend) -> Result<()> {
    let host = crate::toolchain::Host::current();
    match backend {
        ResolvedPreviewBackend::SupportApp(platform) => match platform {
            PreviewPlatform::Ios => {
                toolchain_checks::check_apple(&host, AppleSdk::Ios).await?;
            }
            PreviewPlatform::IosSimulator => {
                toolchain_checks::check_apple(&host, AppleSdk::IosSimulator).await?;
            }
            PreviewPlatform::Macos => {
                toolchain_checks::check_apple(&host, AppleSdk::Macos).await?;
            }
            PreviewPlatform::Android => {
                toolchain_checks::check_android_run(&host).await?;
            }
        },
        ResolvedPreviewBackend::Hydrolysis(_) => {
            toolchain_checks::check_hydrolysis(&host).await?;
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
    fn native_preview_platform_selects_the_host_platform() {
        assert_eq!(
            native_preview_platform_for_os("macos"),
            Some(CliPreviewPlatform::Macos)
        );
        assert_eq!(
            native_preview_platform_for_os("linux"),
            Some(CliPreviewPlatform::Linux)
        );
        assert_eq!(
            native_preview_platform_for_os("windows"),
            Some(CliPreviewPlatform::Windows)
        );
        assert_eq!(native_preview_platform_for_os("freebsd"), None);
    }

    #[test]
    fn native_preview_platform_matches_this_host() {
        let expected = match std::env::consts::OS {
            "macos" => Some(CliPreviewPlatform::Macos),
            "linux" => Some(CliPreviewPlatform::Linux),
            "windows" => Some(CliPreviewPlatform::Windows),
            _ => None,
        };
        match expected {
            Some(platform) => {
                assert_eq!(resolve_preview_platform(None).unwrap(), platform);
            }
            None => assert!(resolve_preview_platform(None).is_err()),
        }
    }

    #[test]
    fn linux_and_windows_default_to_the_hydrolysis_backend() {
        for (platform, target) in [
            (CliPreviewPlatform::Linux, TargetPlatform::Linux),
            (CliPreviewPlatform::Windows, TargetPlatform::Windows),
        ] {
            assert_eq!(
                resolve_preview_backend(platform, None).unwrap(),
                ResolvedPreviewBackend::Hydrolysis(target)
            );
        }
    }

    #[test]
    fn support_app_platforms_resolve_to_their_protocol_platform() {
        for (platform, preview_platform) in [
            (CliPreviewPlatform::Ios, PreviewPlatform::IosSimulator),
            (CliPreviewPlatform::Macos, PreviewPlatform::Macos),
            (CliPreviewPlatform::Android, PreviewPlatform::Android),
        ] {
            assert_eq!(
                resolve_preview_backend(platform, None).unwrap(),
                ResolvedPreviewBackend::SupportApp(preview_platform)
            );
        }
    }

    #[test]
    fn desktop_platform_label_must_match_the_host() {
        // `--platform linux` builds host-native, so on a macOS host it would
        // produce a darwin binary — reject it instead.
        let err = crate::platform::ensure_desktop_platform_is_host(
            CliPreviewPlatform::Linux.desktop_os(),
            "macos",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "`--platform linux` targets the host; this host is macos."
        );
        assert!(
            crate::platform::ensure_desktop_platform_is_host(
                CliPreviewPlatform::Linux.desktop_os(),
                "linux"
            )
            .is_ok()
        );
        // Device platforms carry their own triple and skip the host check.
        assert!(
            crate::platform::ensure_desktop_platform_is_host(
                CliPreviewPlatform::Android.desktop_os(),
                "linux"
            )
            .is_ok()
        );
    }

    #[test]
    fn hydrolysis_test_platform_rejects_device_platforms() {
        assert_eq!(
            resolve_hydrolysis_test_platform(CliPreviewPlatform::Windows).unwrap(),
            TargetPlatform::Windows
        );
        assert!(resolve_hydrolysis_test_platform(CliPreviewPlatform::Ios).is_err());
    }

    #[test]
    fn hydrolysis_preview_theme_defaults_to_material3() {
        assert_eq!(
            resolve_hydrolysis_preview_theme(
                ResolvedPreviewBackend::Hydrolysis(TargetPlatform::MacOS),
                None
            )
            .unwrap(),
            Some(HydrolysisPreviewTheme::Material3)
        );
        assert!(
            resolve_hydrolysis_preview_theme(
                ResolvedPreviewBackend::SupportApp(PreviewPlatform::Macos),
                Some(CliHydrolysisPreviewTheme::Material3)
            )
            .is_err()
        );
    }

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
