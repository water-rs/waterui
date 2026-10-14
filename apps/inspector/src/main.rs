//! The `waterui-inspector` standalone desktop application.
//!
//! Built only when the `desktop` feature is enabled; the binary is a runtime
//! artifact, not a crate component.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
compile_error!("waterui-inspector desktop binary supports macOS, Linux and Windows only");

fn main() {
    let env = waterui::configure_environment!(waterui::env::Environment::new());
    hydrolysis::run(
        waterui_inspector_app::app(env),
        hydrolysis_m3::Material3::defaults(),
    );
}
