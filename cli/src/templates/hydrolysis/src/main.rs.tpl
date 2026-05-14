//! Hydrolysis entry point for {{ ctx.app_display_name }}.

#[cfg(all(feature = "waterui-preview-mode", feature = "waterui-preview-test-mode"))]
compile_error!("enable only one Hydrolysis preview feature at a time");

#[cfg(feature = "waterui-preview-mode")]
mod preview_symbol;

#[cfg(feature = "waterui-preview-mode")]
mod preview_runtime;

#[cfg(feature = "waterui-preview-test-mode")]
mod preview_test;

#[cfg(feature = "waterui-preview-test-mode")]
mod preview_test_runtime;

#[cfg(all(feature = "waterui-preview-mode", not(feature = "waterui-preview-test-mode")))]
fn main() {
    preview_runtime::run();
}

#[cfg(all(feature = "waterui-preview-test-mode", not(feature = "waterui-preview-mode")))]
fn main() {
    preview_test_runtime::run();
}

#[cfg(not(any(feature = "waterui-preview-mode", feature = "waterui-preview-test-mode")))]
fn main() {
    let mut env = waterui::configure_environment!(waterui::env::Environment::new());
    hydrolysis_m3::install(&mut env);
    let app = {{ ctx.crate_name_ident() }}::app(env);
    hydrolysis::run(app);
}
