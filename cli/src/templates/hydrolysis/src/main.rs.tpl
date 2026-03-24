//! Hydrolysis entry point for {{ ctx.app_display_name }}.

#[cfg(feature = "waterui-preview-mode")]
mod preview_symbol;

#[cfg(feature = "waterui-preview-mode")]
mod preview_runtime;

#[cfg(feature = "waterui-preview-mode")]
fn main() {
    preview_runtime::run();
}

#[cfg(not(feature = "waterui-preview-mode"))]
fn main() {
    let app = {{ ctx.crate_name_ident() }}::app(waterui::configure_environment!(waterui::env::Environment::new()));
    hydrolysis::run(app);
}
