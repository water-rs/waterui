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

pub use launcher::{PreviewSession, launch_preview_session};
pub use protocol::{PreviewPlatform, Size};
pub use request::{
    CliHydrolysisPreviewTheme, CliPreviewBackend, CliPreviewPlatform, PreviewRequest, PreviewTarget,
};

/// The cache root preview support assets live under on `host`:
/// `WATER_CACHE_DIR` when set, the host's cache directory joined with
/// `waterui`, else `waterui-cache` in the host's temporary directory. The
/// spawned support app is handed this root through `WATER_CACHE_DIR`, so it
/// registers where the CLI watches, whatever its own default would be.
fn water_cache_dir(host: &crate::toolchain::Host) -> std::path::PathBuf {
    if let Some(dir) = host.env("WATER_CACHE_DIR") {
        return std::path::PathBuf::from(dir);
    }
    if let Some(cache_dir) = host.cache_dir() {
        return cache_dir.join("waterui");
    }
    host.temp_dir().join("waterui-cache")
}

/// Root of the preview support assets on `host`.
fn preview_cache_root_dir(host: &crate::toolchain::Host) -> std::path::PathBuf {
    waterui_preview_protocol::registry::preview_cache_root_dir_in(&water_cache_dir(host))
}

/// Directory the CLI watches for preview support app registrations on `host`.
pub(crate) fn preview_instance_registry_dir(host: &crate::toolchain::Host) -> std::path::PathBuf {
    waterui_preview_protocol::registry::preview_instance_registry_dir_in(&preview_cache_root_dir(
        host,
    ))
}

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
