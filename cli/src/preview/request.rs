//! Shared preview request resolution for `water preview` and the MCP
//! `preview` tool.
//!
//! Both entry points accept the same arguments — a `#[preview]` function path
//! or expression target, a frame size, and optional platform/backend/theme
//! overrides — and resolve them through the functions here, so the two can
//! never drift apart.

use std::time::Duration;

use clap::ValueEnum;
use eyre::{Result, bail};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::platform::{TargetBackend, TargetPlatform};
use crate::preview::protocol::function_path_to_symbol;
use crate::preview::{HydrolysisPreviewTheme, PreviewSource};
use crate::project::{Manifest, resolve_backend};
use crate::toolchain_checks;
pub use waterui_preview_protocol::run::DEFAULT_CONTENT_DEADLINE;

/// Default frame size shared by `water preview --frame` and the MCP `preview`
/// tool's `frame` argument.
pub const DEFAULT_FRAME: &str = "375x667";

/// Target platform for preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CliPreviewPlatform {
    /// iOS — with the Apple backend, the `UIKit` realization rendered on
    /// this Mac through Mac Catalyst.
    Ios,
    /// macOS.
    Macos,
    /// Android device — Hydrolysis inside the preview host instrumentation.
    Android,
    /// Linux (Hydrolysis).
    Linux,
    /// Windows (Hydrolysis).
    Windows,
}

impl CliPreviewPlatform {
    /// The desktop OS this platform renders on, if it is one. iOS and
    /// Android previews carry their own triples and return `None`.
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
#[non_exhaustive]
pub enum CliPreviewBackend {
    /// Apple native backend — `AppKit` on macOS, `UIKit` through Mac
    /// Catalyst on iOS.
    Apple,
    /// Hydrolysis direct renderer — the desktop binary for desktop
    /// platforms, the preview host instrumentation for Android.
    Hydrolysis,
}

/// The preview execution path `resolve_preview_backend` resolved, carrying
/// the target each path builds for.
///
/// A Hydrolysis render compiles a managed backend for a desktop
/// [`TargetPlatform`] — or for `TargetPlatform::Android`, where the launcher
/// cdylib registers `preview_runtime::run` on `JNI_OnLoad` inside the
/// preview host's instrumentation; an Apple render builds the in-process
/// preview binary for the [`TargetPlatform`] it carries. Carrying the
/// target on the variant keeps call sites from re-deriving it from the CLI
/// platform label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResolvedPreviewBackend {
    /// Hydrolysis render for the given target — a desktop binary, or the
    /// Android preview host's instrumentation for `TargetPlatform::Android`.
    Hydrolysis(TargetPlatform),
    /// The in-process Apple preview binary, built for the given target and
    /// exec'd on this Mac: `TargetPlatform::MacOS` renders the `AppKit`
    /// realization, `TargetPlatform::MacCatalyst` the `UIKit` one under the
    /// iPad idiom — `water preview --platform ios`, with no simulator.
    Apple(TargetPlatform),
}

/// Theme package for Hydrolysis preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
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
#[non_exhaustive]
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

    /// The [`PreviewSource`] for this target.
    #[must_use]
    pub fn source(&self) -> PreviewSource<'_> {
        match self {
            Self::Function { symbol, .. } => PreviewSource::Symbol(symbol),
            Self::Expression { expression } => PreviewSource::Expression(expression),
        }
    }
}

/// A fully resolved preview render, ready to hand to the Hydrolysis or
/// Apple execution path.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewRequest {
    /// Resolved target platform.
    platform: CliPreviewPlatform,
    /// Resolved rendering backend and the target it builds for or serves.
    backend: ResolvedPreviewBackend,
    /// Hydrolysis theme package — `Some` iff `backend` is
    /// [`ResolvedPreviewBackend::Hydrolysis`].
    hydrolysis_theme: Option<HydrolysisPreviewTheme>,
    /// What to render.
    target: PreviewTarget,
    /// Frame width in logical units.
    width: f32,
    /// Frame height in logical units.
    height: f32,
    /// How long a capture waits for the content its views owe — a decoding
    /// image, a `Suspense` future — before the preview fails naming them.
    content_deadline: Duration,
}
impl PreviewRequest {
    /// A `PreviewRequest` carrying each of its fields.
    #[must_use]
    pub const fn new(
        platform: CliPreviewPlatform,
        backend: ResolvedPreviewBackend,
        hydrolysis_theme: Option<HydrolysisPreviewTheme>,
        target: PreviewTarget,
        width: f32,
        height: f32,
        content_deadline: Duration,
    ) -> Self {
        Self {
            platform,
            backend,
            hydrolysis_theme,
            target,
            width,
            height,
            content_deadline,
        }
    }

    /// Resolved target platform.
    #[must_use]
    pub const fn platform(&self) -> CliPreviewPlatform {
        self.platform
    }

    /// Resolved rendering backend and the target it builds for or serves.
    #[must_use]
    pub const fn backend(&self) -> ResolvedPreviewBackend {
        self.backend
    }

    /// Hydrolysis theme package — `Some` iff `backend` is
    /// [`ResolvedPreviewBackend::Hydrolysis`].
    #[must_use]
    pub const fn hydrolysis_theme(&self) -> Option<HydrolysisPreviewTheme> {
        self.hydrolysis_theme
    }

    /// What to render.
    #[must_use]
    pub const fn target(&self) -> &PreviewTarget {
        &self.target
    }

    /// Frame width in logical units.
    #[must_use]
    pub const fn width(&self) -> f32 {
        self.width
    }

    /// Frame height in logical units.
    #[must_use]
    pub const fn height(&self) -> f32 {
        self.height
    }

    /// How long a capture waits for the content its views owe — a decoding
    /// image, a `Suspense` future — before the preview fails naming them.
    #[must_use]
    pub const fn content_deadline(&self) -> Duration {
        self.content_deadline
    }
}

/// Parse a content timeout given in seconds — how long a capture waits for
/// the content its views owe before the preview fails naming them. `None`
/// takes the protocol's default.
///
/// # Errors
/// Returns an error if the timeout is not a positive finite number of
/// seconds.
pub fn parse_content_timeout(seconds: Option<f64>) -> Result<Duration> {
    let Some(seconds) = seconds else {
        return Ok(DEFAULT_CONTENT_DEADLINE);
    };
    if !seconds.is_finite() || seconds <= 0.0 {
        bail!("Invalid content timeout: must be a positive finite number of seconds");
    }
    Duration::try_from_secs_f64(seconds)
        .map_err(|error| eyre::eyre!("Invalid content timeout of {seconds}s: {error}"))
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
/// [`TargetPlatform`] Hydrolysis builds for, or the Apple target the
/// in-process Apple preview binary builds for.
///
/// # Errors
/// Returns an error for a conflicting project declaration or a backend
/// without preview support on this platform.
pub fn resolve_preview_backend(
    project: &Manifest,
    platform: CliPreviewPlatform,
    backend_override: Option<CliPreviewBackend>,
) -> Result<ResolvedPreviewBackend> {
    let target = match platform {
        CliPreviewPlatform::Ios => TargetPlatform::IOS,
        CliPreviewPlatform::Macos => TargetPlatform::MacOS,
        CliPreviewPlatform::Android => TargetPlatform::Android,
        CliPreviewPlatform::Linux => TargetPlatform::Linux,
        CliPreviewPlatform::Windows => TargetPlatform::Windows,
    };
    let backend = resolve_backend(
        project,
        target,
        backend_override.map(|backend| match backend {
            CliPreviewBackend::Apple => TargetBackend::Apple,
            CliPreviewBackend::Hydrolysis => TargetBackend::Hydrolysis,
        }),
    )?;

    Ok(match (platform, backend) {
        (CliPreviewPlatform::Ios, TargetBackend::Apple) => {
            ResolvedPreviewBackend::Apple(TargetPlatform::MacCatalyst)
        }
        (CliPreviewPlatform::Macos, TargetBackend::Apple) => {
            ResolvedPreviewBackend::Apple(TargetPlatform::MacOS)
        }
        (CliPreviewPlatform::Macos, TargetBackend::Hydrolysis) => {
            ResolvedPreviewBackend::Hydrolysis(TargetPlatform::MacOS)
        }
        (CliPreviewPlatform::Linux, TargetBackend::Hydrolysis) => {
            ResolvedPreviewBackend::Hydrolysis(TargetPlatform::Linux)
        }
        (CliPreviewPlatform::Windows, TargetBackend::Hydrolysis) => {
            ResolvedPreviewBackend::Hydrolysis(TargetPlatform::Windows)
        }
        (CliPreviewPlatform::Android, TargetBackend::Hydrolysis) => {
            ResolvedPreviewBackend::Hydrolysis(TargetPlatform::Android)
        }
        (platform, backend) => {
            bail!(
                "Preview backend {backend:?} does not support platform {platform:?}. Valid combinations: ios/apple, macos/apple, macos/hydrolysis, linux/hydrolysis, windows/hydrolysis, android/hydrolysis"
            );
        }
    })
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

/// The preview platform for each supported desktop host OS.
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
pub fn resolve_hydrolysis_test_platform(
    project: &Manifest,
    platform: CliPreviewPlatform,
) -> Result<TargetPlatform> {
    match resolve_preview_backend(project, platform, Some(CliPreviewBackend::Hydrolysis))? {
        ResolvedPreviewBackend::Hydrolysis(TargetPlatform::Android) => Err(eyre::eyre!(
            "`water preview test` runs on desktop Hydrolysis only; \
             --platform android renders through the preview host instrumentation"
        )),
        ResolvedPreviewBackend::Hydrolysis(target) => Ok(target),
        ResolvedPreviewBackend::Apple(_) => {
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
/// Returns the Kotlin toolchain an Android check resolved — the preview
/// build hands it to every consumer rather than probing `kotlinc` again for
/// the same invocation. `None` on every non-Android backend.
///
/// # Errors
/// Returns an error if a required toolchain component is missing.
pub async fn check_toolchain_for_backend(
    host: &crate::toolchain::Host,
    backend: ResolvedPreviewBackend,
) -> Result<Option<crate::android::KotlinToolchain>> {
    match backend {
        ResolvedPreviewBackend::Apple(platform) => {
            toolchain_checks::check_apple_preview(host, platform).await?;
        }
        ResolvedPreviewBackend::Hydrolysis(TargetPlatform::Android) => {
            return toolchain_checks::check_android_run(host).await.map(Some);
        }
        ResolvedPreviewBackend::Hydrolysis(_) => {
            toolchain_checks::check_hydrolysis(host).await?;
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    #[test]
    fn preview_declarations_conflicts_and_unsupported_defaults() {
        let mut project = project_manifest();
        project.set_platforms(std::collections::BTreeMap::new());
        assert!(resolve_preview_backend(&project, CliPreviewPlatform::Linux, None).is_err());
        project.set_platforms(std::collections::BTreeMap::from([(
            crate::project::PlatformName::Macos,
            crate::project::PlatformConfig::new(TargetBackend::Hydrolysis),
        )]));
        assert_eq!(
            resolve_preview_backend(&project, CliPreviewPlatform::Macos, None).unwrap(),
            ResolvedPreviewBackend::Hydrolysis(TargetPlatform::MacOS)
        );
        let error = resolve_preview_backend(
            &project,
            CliPreviewPlatform::Macos,
            Some(CliPreviewBackend::Apple),
        )
        .unwrap_err();
        assert!(error.to_string().contains("conflicts"));
        assert!(
            resolve_hydrolysis_test_platform(
                &project_manifest_with_apple(),
                CliPreviewPlatform::Macos
            )
            .is_err()
        );
    }

    fn project_manifest_with_apple() -> Manifest {
        Manifest::parse("[package]\nname = 'Demo'\nbundle_identifier = 'dev.example.demo'\n[platforms.macos]\nbackend = 'apple'").unwrap()
    }

    fn project_manifest() -> Manifest {
        Manifest::parse("[package]\nname = 'Demo'\nbundle_identifier = 'dev.example.demo'\n[platforms.linux]\nbackend = 'hydrolysis'").unwrap()
    }

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
    fn linux_declaration_and_windows_default_use_hydrolysis() {
        for (platform, target) in [
            (CliPreviewPlatform::Linux, TargetPlatform::Linux),
            (CliPreviewPlatform::Windows, TargetPlatform::Windows),
        ] {
            assert_eq!(
                resolve_preview_backend(&project_manifest(), platform, None).unwrap(),
                ResolvedPreviewBackend::Hydrolysis(target)
            );
        }
    }

    #[test]
    fn apple_platforms_resolve_to_the_in_process_preview_binary() {
        assert_eq!(
            resolve_preview_backend(&project_manifest(), CliPreviewPlatform::Macos, None).unwrap(),
            ResolvedPreviewBackend::Apple(TargetPlatform::MacOS)
        );
        // `--platform ios` renders the UIKit realization on this Mac through
        // Mac Catalyst — the default backend, an explicit `apple`, and an
        // `[platforms.ios]` declaration all resolve there.
        let catalyst = ResolvedPreviewBackend::Apple(TargetPlatform::MacCatalyst);
        assert_eq!(
            resolve_preview_backend(&project_manifest(), CliPreviewPlatform::Ios, None).unwrap(),
            catalyst
        );
        assert_eq!(
            resolve_preview_backend(
                &project_manifest(),
                CliPreviewPlatform::Ios,
                Some(CliPreviewBackend::Apple)
            )
            .unwrap(),
            catalyst
        );
        let declared = Manifest::parse(
            "[package]\nname = 'Demo'\nbundle_identifier = 'dev.example.demo'\n[platforms.ios]\nbackend = 'apple'",
        )
        .unwrap();
        assert_eq!(
            resolve_preview_backend(&declared, CliPreviewPlatform::Ios, None).unwrap(),
            catalyst
        );
        assert!(
            resolve_preview_backend(
                &project_manifest(),
                CliPreviewPlatform::Ios,
                Some(CliPreviewBackend::Hydrolysis)
            )
            .is_err()
        );
    }

    #[test]
    fn android_resolves_to_the_hydrolysis_preview_host() {
        assert_eq!(
            resolve_preview_backend(&project_manifest(), CliPreviewPlatform::Android, None)
                .unwrap(),
            ResolvedPreviewBackend::Hydrolysis(TargetPlatform::Android)
        );
        assert_eq!(
            resolve_preview_backend(
                &project_manifest(),
                CliPreviewPlatform::Android,
                Some(CliPreviewBackend::Hydrolysis)
            )
            .unwrap(),
            ResolvedPreviewBackend::Hydrolysis(TargetPlatform::Android)
        );
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
            resolve_hydrolysis_test_platform(&project_manifest(), CliPreviewPlatform::Windows)
                .unwrap(),
            TargetPlatform::Windows
        );
        assert!(
            resolve_hydrolysis_test_platform(&project_manifest(), CliPreviewPlatform::Ios).is_err()
        );
        assert!(
            resolve_hydrolysis_test_platform(&project_manifest(), CliPreviewPlatform::Android)
                .is_err()
        );
    }

    /// `water preview --platform android` probes `kotlinc -version` once: the
    /// launcher build's Android build context consumes the toolchain the
    /// preview's toolchain check resolved instead of probing again (every
    /// probe launches a JVM). The fake tools log each invocation to
    /// `WATERUI_FAKE_LOG`.
    #[test]
    #[cfg(unix)]
    fn android_preview_probes_kotlinc_once() {
        use crate::android::ndk_version::ANDROID_NDK_VERSION;
        use crate::android::platform::{AndroidAbi, resolve_android_build_context};
        use crate::toolchain::testing::TestMachine;
        use std::ffi::OsString;
        use std::path::Path;

        // The NDK host tag the build context resolves on the unix hosts the
        // CLI supports (Apple support is ARM64-only).
        const NDK_HOST_TAG: &str = if cfg!(target_os = "macos") {
            "darwin-arm64"
        } else {
            "linux-x86_64"
        };
        const API_LEVEL: u32 = 35;

        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        machine.install_android_platform("android-36");
        machine.install_android_build_tools("36.0.0");
        machine.install_android_ndk(ANDROID_NDK_VERSION);
        // The build context requires the API-level compiler wrappers under
        // the host tag's `prebuilt/<tag>/bin`.
        for tool in ["clang", "clang++"] {
            machine.executable(
                Path::new("sdk/ndk")
                    .join(ANDROID_NDK_VERSION)
                    .join("toolchains/llvm/prebuilt")
                    .join(NDK_HOST_TAG)
                    .join("bin")
                    .join(format!("aarch64-linux-android{API_LEVEL}-{tool}")),
            );
        }
        machine.install("java");
        machine.install("kotlinc");
        machine.install("cmake");
        machine.install("rustup");
        machine.respond(
            "RUSTUP_ACTIVE_TOOLCHAIN",
            "stable-aarch64-unknown-fake (default)",
        );
        machine.respond(
            "RUSTUP_INSTALLED_TARGETS",
            "aarch64-linux-android\narmv7-linux-androideabi\ni686-linux-android\nx86_64-linux-android",
        );

        let fake_log = machine.root().join("tools.log");
        let host = machine.host([
            (OsString::from("ANDROID_SDK_ROOT"), sdk.into_os_string()),
            (
                OsString::from("WATERUI_FAKE_KOTLINC_VERSION"),
                OsString::from(crate::build_info::ANDROID_KOTLIN_VERSION),
            ),
            (
                OsString::from("WATERUI_FAKE_LOG"),
                fake_log.clone().into_os_string(),
            ),
        ]);

        smol::block_on(async {
            // The gate `water preview` runs, then the build-context
            // resolution the launcher's cargo build performs.
            let kotlin = check_toolchain_for_backend(
                &host,
                ResolvedPreviewBackend::Hydrolysis(TargetPlatform::Android),
            )
            .await
            .expect("the fake host satisfies the Android preview toolchain")
            .expect("the Android preview check resolves a Kotlin toolchain");
            let abi = AndroidAbi::Arm64V8a;
            resolve_android_build_context(&host, abi, &abi.triple(), API_LEVEL, &kotlin)
                .await
                .expect("the fake host satisfies the Android build context");
        });

        let log = std::fs::read_to_string(&fake_log).expect("read the fake tool log");
        let kotlinc_runs = log
            .lines()
            .filter(|line| line.split_whitespace().next() == Some("kotlinc"))
            .count();
        assert_eq!(
            kotlinc_runs, 1,
            "the Android preview's toolchain resolution must probe kotlinc exactly once:\n{log}"
        );
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
        assert_eq!(
            resolve_hydrolysis_preview_theme(
                ResolvedPreviewBackend::Hydrolysis(TargetPlatform::Android),
                None
            )
            .unwrap(),
            Some(HydrolysisPreviewTheme::Material3)
        );
        assert!(
            resolve_hydrolysis_preview_theme(
                ResolvedPreviewBackend::Apple(TargetPlatform::MacCatalyst),
                Some(CliHydrolysisPreviewTheme::Material3)
            )
            .is_err()
        );
    }
}
