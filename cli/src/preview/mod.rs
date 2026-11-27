//! Preview system for rendering and capturing `WaterUI` views.
//!
//! This module provides infrastructure for the `water preview` command:
//!
//! - `apple`: the in-process Apple preview binary (`AppKit` on macOS,
//!   headless `UIKit` on `aarch64-apple-ios-sim` for `--platform ios`)
//! - `ios_sim`: headless iOS Simulator preview host and helper cache
//! - `hydrolysis`: the desktop Hydrolysis preview binary, and
//!   `hydrolysis_android`: the Android preview host's instrumentation
//! - [`protocol`]: the `waterui_preview_*` export symbol naming
//! - `run`: the `WATERUI_PREVIEW_RUN_CONFIG` channel every generated
//!   preview binary reads
//! - [`request`]: Preview argument resolution shared by `water preview` and
//!   the `water mcp` `preview` tool

pub(crate) mod apple;
pub(crate) mod hydrolysis;
pub(crate) mod hydrolysis_android;
pub(crate) mod ios_sim;
pub mod protocol;
pub mod request;
pub(crate) mod run;
pub mod video;

use std::path::{Path, PathBuf};

use askama::Template;
use eyre::{Result, bail};

pub use apple::{
    ApplePreviewBinary, ApplePreviewRenderOptions, ApplePreviewRequest, build_preview_with_apple,
    render_preview_with_apple,
};
pub use hydrolysis::{
    HydrolysisPreviewRequest, HydrolysisPreviewTheme, discover_hydrolysis_preview_exports,
    render_preview_with_hydrolysis, test_preview_with_hydrolysis,
};

pub use request::{
    CliHydrolysisPreviewTheme, CliPreviewBackend, CliPreviewColorScheme, CliPreviewPlatform,
    PreviewColorScheme, PreviewRequest, PreviewTarget,
};
pub use video::{DEFAULT_VIDEO_FPS, HardwareVideoEncoder, encode_raw_frame_stream_to_mp4};
pub use waterui_preview_protocol::run::{
    ScenarioEvent as PreviewScenarioEvent, ScenarioEventKind as PreviewEventKind,
    ScenarioPointerButton as PreviewPointerButton,
};

/// Video recording output options (`--video` and `--fps`) shared by
/// `water preview --scenario` and `water preview test`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewVideoOptions {
    /// Destination `.mp4` path.
    output_path: PathBuf,
    /// Fixed sampling frame rate in frames per second (strictly positive).
    fps: u32,
}

impl PreviewVideoOptions {
    /// Validates and constructs video output options (`fps` defaults to [`DEFAULT_VIDEO_FPS`]).
    ///
    /// # Errors
    ///
    /// Returns an error if `fps` is zero.
    pub fn new(output_path: PathBuf, fps: u32) -> Result<Self> {
        if fps == 0 {
            bail!("`--fps` must be greater than zero.");
        }
        Ok(Self { output_path, fps })
    }

    /// Destination `.mp4` path.
    #[must_use]
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    /// Fixed sampling frame rate in frames per second.
    #[must_use]
    pub const fn fps(&self) -> u32 {
        self.fps
    }
}

/// Interactive capture scenario shared by Hydrolysis and Apple previews.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewScenario {
    /// Capture timestamps in milliseconds from scenario start for PNG frames.
    captures_ms: Vec<u64>,
    /// Input events sorted by timestamp.
    events: Vec<PreviewScenarioEvent>,
    /// Directory receiving `frame-XXXXms.png` captures, when PNG output is requested.
    output_dir: Option<PathBuf>,
    /// Optional explicit timeline duration in milliseconds for video recording.
    duration_ms: Option<u64>,
    /// Optional hardware-encoded MP4 output configuration.
    video: Option<PreviewVideoOptions>,
}

impl PreviewScenario {
    /// Constructs a scenario replaying `events` and producing PNG frames in
    /// `output_dir` and/or a hardware-encoded MP4 video via `video`.
    #[must_use]
    pub const fn new(
        captures_ms: Vec<u64>,
        events: Vec<PreviewScenarioEvent>,
        output_dir: Option<PathBuf>,
        duration_ms: Option<u64>,
        video: Option<PreviewVideoOptions>,
    ) -> Self {
        Self {
            captures_ms,
            events,
            output_dir,
            duration_ms,
            video,
        }
    }

    /// Capture timestamps in milliseconds from scenario start.
    #[must_use]
    pub fn captures_ms(&self) -> &[u64] {
        &self.captures_ms
    }

    /// Input events sorted by timestamp.
    #[must_use]
    pub fn events(&self) -> &[PreviewScenarioEvent] {
        &self.events
    }

    /// Directory where PNG frames are written, if requested.
    #[must_use]
    pub fn output_dir(&self) -> Option<&Path> {
        self.output_dir.as_deref()
    }

    /// Explicit scenario duration in milliseconds for video recording, if set.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// Hardware-encoded MP4 output options, if video recording is requested.
    #[must_use]
    pub const fn video(&self) -> Option<&PreviewVideoOptions> {
        self.video.as_ref()
    }
}

/// Source used to produce a preview view — an existing `#[preview]` export
/// or an inline `WaterUI` expression. Shared by every preview backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PreviewSource<'a> {
    /// Existing `#[preview]` export symbol.
    Symbol(&'a str),
    /// Inline Rust expression returning `impl View`.
    Expression(&'a str),
}

/// The `preview_target.rs` template every generated preview binary's
/// target module renders from. The Apple preview package's module and the
/// Hydrolysis backend's preview bindings differ only in the extern symbol
/// symbol-mode loads through and in the Hydrolysis-only runtime helpers —
/// `preview_style`, `app_environment` and the semantic automation body —
/// which the Apple render leaves unset.
#[derive(Template)]
#[template(path = "src/templates/preview_target.rs.tpl", escape = "none")]
pub(crate) struct PreviewTargetTemplate<'a> {
    backend_label: &'a str,
    extern_fn_name: &'a str,
    expression_mode: bool,
    preview_symbol: &'a str,
    preview_expression: &'a str,
    crate_name_ident: &'a str,
    preview_theme_style: Option<&'a str>,
    semantic_automation_body: Option<&'a str>,
    offscreen_semantic_test: bool,
}

impl<'a> PreviewTargetTemplate<'a> {
    /// The Apple preview package's `preview_target.rs`.
    pub(crate) const fn apple(source: PreviewSource<'a>, crate_name_ident: &'a str) -> Self {
        Self::new(
            "Apple",
            "waterui_apple_preview_entry",
            source,
            crate_name_ident,
            None,
            None,
            false,
        )
    }

    /// The Hydrolysis backend's preview bindings — `preview_symbol.rs`, or
    /// `preview_test.rs` when `automation_body` runs a semantic test.
    pub(crate) const fn hydrolysis(
        source: PreviewSource<'a>,
        crate_name_ident: &'a str,
        theme_style: &'a str,
        automation_body: Option<&'a str>,
        offscreen_semantic_test: bool,
    ) -> Self {
        Self::new(
            "Hydrolysis",
            "waterui_hydrolysis_preview_entry",
            source,
            crate_name_ident,
            Some(theme_style),
            automation_body,
            offscreen_semantic_test,
        )
    }

    const fn new(
        backend_label: &'a str,
        extern_fn_name: &'a str,
        source: PreviewSource<'a>,
        crate_name_ident: &'a str,
        preview_theme_style: Option<&'a str>,
        semantic_automation_body: Option<&'a str>,
        offscreen_semantic_test: bool,
    ) -> Self {
        let (expression_mode, preview_symbol, preview_expression) = match source {
            PreviewSource::Symbol(symbol) => (false, symbol, ""),
            PreviewSource::Expression(expression) => (true, "", expression),
        };
        Self {
            backend_label,
            extern_fn_name,
            expression_mode,
            preview_symbol,
            preview_expression,
            crate_name_ident,
            preview_theme_style,
            semantic_automation_body,
            offscreen_semantic_test,
        }
    }
}

#[cfg(test)]
mod tests {
    use askama::Template as _;

    use super::{PreviewSource, PreviewTargetTemplate};

    #[test]
    fn offscreen_preview_target_template_wires_hdr_recording() {
        let rendered = PreviewTargetTemplate::hydrolysis(
            PreviewSource::Symbol("waterui_preview_demo"),
            "demo_app",
            "hydrolysis_m3::Material3::defaults()",
            Some("let _ = app.snapshot();"),
            true,
        )
        .render()
        .expect("render offscreen preview target template");
        assert!(rendered.contains("hdr: bool"));
        assert!(rendered.contains(".hdr(hdr)"));
        assert!(rendered.contains("RawFrameStreamWriter::create_hdr"));
        assert!(rendered.contains("start_hdr_frame_recording"));
        assert!(rendered.contains("write_hdr_frame_f32"));
    }

    #[test]
    fn hydrolysis_preview_target_template_extracts_env_via_into_parts() {
        let rendered = PreviewTargetTemplate::hydrolysis(
            PreviewSource::Symbol("waterui_preview_demo"),
            "demo_app",
            "hydrolysis_m3::Material3::defaults()",
            None,
            false,
        )
        .render()
        .expect("render hydrolysis preview target template");
        assert!(rendered.contains("demo_app::app(env).into_parts().env"));
        assert!(!rendered.contains("demo_app::app(env).env"));
    }
}
