//! Preview system for rendering and capturing `WaterUI` views.
//!
//! This module provides infrastructure for the `water preview` command:
//!
//! - `app_client`: TCP client for communicating with the preview support app
//! - [`protocol`]: Message definitions for preview support app communication
//! - `inputs`: Content fingerprinting for preview build caching
//! - `launcher`: Preview support app lifecycle management
//! - [`request`]: Preview argument resolution shared by `water preview` and
//!   the `water mcp` `preview` tool

mod app_client;
pub(crate) mod apple;
pub(crate) mod hydrolysis;
pub(crate) mod hydrolysis_android;
mod inputs;
mod launcher;
pub mod protocol;
pub mod request;
pub(crate) mod run;

use askama::Template;

pub use app_client::{PreviewAppClient, PreviewProbe};
pub use apple::{ApplePreviewRequest, render_preview_with_apple};
pub use hydrolysis::{
    HydrolysisPreviewEventKind, HydrolysisPreviewPointerButton, HydrolysisPreviewRequest,
    HydrolysisPreviewScenario, HydrolysisPreviewScenarioEvent, HydrolysisPreviewTheme,
    discover_hydrolysis_preview_exports, render_preview_with_hydrolysis,
    test_preview_with_hydrolysis,
};
pub use hydrolysis_android::{
    HydrolysisAndroidPreviewRequest, render_preview_with_hydrolysis_android,
};
pub use launcher::{PreviewSession, launch_preview_session};
pub use protocol::{PreviewPlatform, Size};
pub use request::{
    CliHydrolysisPreviewTheme, CliPreviewBackend, CliPreviewPlatform, PreviewRequest, PreviewTarget,
};

/// Source used to produce a preview view — an existing `#[preview]` export
/// or an inline `WaterUI` expression. Shared by every preview backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        )
    }

    /// The Hydrolysis backend's preview bindings — `preview_symbol.rs`, or
    /// `preview_test.rs` when `automation_body` runs a semantic test.
    pub(crate) const fn hydrolysis(
        source: PreviewSource<'a>,
        crate_name_ident: &'a str,
        theme_style: &'a str,
        automation_body: Option<&'a str>,
    ) -> Self {
        Self::new(
            "Hydrolysis",
            "waterui_hydrolysis_preview_entry",
            source,
            crate_name_ident,
            Some(theme_style),
            automation_body,
        )
    }

    const fn new(
        backend_label: &'a str,
        extern_fn_name: &'a str,
        source: PreviewSource<'a>,
        crate_name_ident: &'a str,
        preview_theme_style: Option<&'a str>,
        semantic_automation_body: Option<&'a str>,
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
        }
    }
}
