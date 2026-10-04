//! Hydrolysis entry point for {{ ctx.app_display_name }}.

#[cfg(all(feature = "waterui-preview-mode", feature = "waterui-preview-test-mode"))]
compile_error!("enable only one Hydrolysis run feature at a time");

#[cfg(all(feature = "waterui-preview-mode", feature = "waterui-mcp-mode"))]
compile_error!("enable only one Hydrolysis run feature at a time");

#[cfg(all(feature = "waterui-preview-test-mode", feature = "waterui-mcp-mode"))]
compile_error!("enable only one Hydrolysis run feature at a time");

#[cfg(any(
    feature = "waterui-preview-mode",
    feature = "waterui-preview-test-mode",
    feature = "waterui-mcp-mode"
))]
mod run_config;

#[cfg(feature = "waterui-preview-mode")]
mod preview_symbol;

#[cfg(feature = "waterui-preview-mode")]
mod preview_runtime;

#[cfg(feature = "waterui-preview-test-mode")]
mod preview_test;

#[cfg(feature = "waterui-preview-test-mode")]
mod preview_test_runtime;

#[cfg(feature = "waterui-mcp-mode")]
mod mcp_runtime;

#[cfg(all(
    feature = "waterui-preview-mode",
    not(feature = "waterui-preview-test-mode"),
    not(feature = "waterui-mcp-mode")
))]
fn main() {
    preview_runtime::run();
}

#[cfg(all(
    feature = "waterui-preview-test-mode",
    not(feature = "waterui-preview-mode"),
    not(feature = "waterui-mcp-mode")
))]
fn main() {
    preview_test_runtime::run();
}

#[cfg(all(
    feature = "waterui-mcp-mode",
    not(feature = "waterui-preview-mode"),
    not(feature = "waterui-preview-test-mode")
))]
fn main() {
    mcp_runtime::run();
}

#[cfg(not(any(
    feature = "waterui-preview-mode",
    feature = "waterui-preview-test-mode",
    feature = "waterui-mcp-mode"
)))]
fn main() {
    let env = waterui::configure_environment!(waterui::env::Environment::new());
    let app = {{ ctx.crate_name_ident() }}::app(env);
    hydrolysis::run(app, hydrolysis_m3::Material3::defaults());
}
