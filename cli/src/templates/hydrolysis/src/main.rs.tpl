//! Hydrolysis entry point for __APP_DISPLAY_NAME__.

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
    let app = __CRATE_NAME_IDENT__::app(waterui::configure_environment!(waterui::env::Environment::new()));
    hydrolysis::run(app);
}
