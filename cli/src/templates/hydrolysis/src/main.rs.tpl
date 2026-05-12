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
    let mut env = waterui::configure_environment!(waterui::env::Environment::new());
    hydrolysis_m3::install(&mut env);
    let app = {{ ctx.crate_name_ident() }}::app(env);
    hydrolysis::run(app);
}
