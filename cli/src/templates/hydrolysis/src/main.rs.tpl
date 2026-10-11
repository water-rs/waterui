//! Hydrolysis entry point for {{ ctx.app_display_name }}.

// Windows builds link the GUI subsystem: a console window must not appear
// beside the application window, and launch tooling such as
// `WaitForInputIdle` treats console-subsystem executables as non-GUI.
// `water run` still streams the app's stdout and stderr because inherited
// pipe handles work without a console; the preview, preview-test and MCP
// mains below share this crate root and need only stdio pipes.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(all(feature = "waterui-preview-mode", feature = "waterui-preview-test-mode"))]
compile_error!("enable only one Hydrolysis run feature at a time");

#[cfg(all(feature = "waterui-preview-mode", feature = "waterui-mcp-mode"))]
compile_error!("enable only one Hydrolysis run feature at a time");

#[cfg(all(feature = "waterui-preview-test-mode", feature = "waterui-mcp-mode"))]
compile_error!("enable only one Hydrolysis run feature at a time");

#[cfg(feature = "waterui-mcp-mode")]
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

{% if ctx.cef_runtime_enabled() %}
/// Installs the macOS sandbox and the `CefAppProtocol` `NSApplication`
/// subclass the CEF runtime requires. Runs before `app(env)` installs the
/// engine and before any framework requests the shared application.
fn initialize_cef_runtime() {
    #[cfg(target_os = "macos")]
    {
        waterui_browser_cef::initialize_sandbox_early();
        waterui_browser_cef::initialize_macos_application();
    }
}

{% endif %}
#[cfg(all(
    feature = "waterui-preview-mode",
    not(feature = "waterui-preview-test-mode"),
    not(feature = "waterui-mcp-mode")
))]
fn main() {
    {% if ctx.cef_runtime_enabled() %}
    initialize_cef_runtime();
    {% endif %}
    preview_runtime::run();
}

#[cfg(all(
    feature = "waterui-preview-test-mode",
    not(feature = "waterui-preview-mode"),
    not(feature = "waterui-mcp-mode")
))]
fn main() {
    {% if ctx.cef_runtime_enabled() %}
    initialize_cef_runtime();
    {% endif %}
    preview_test_runtime::run();
}

#[cfg(all(
    feature = "waterui-mcp-mode",
    not(feature = "waterui-preview-mode"),
    not(feature = "waterui-preview-test-mode")
))]
fn main() {
    {% if ctx.cef_runtime_enabled() %}
    initialize_cef_runtime();
    {% endif %}
    mcp_runtime::run();
}

#[cfg(not(any(
    feature = "waterui-preview-mode",
    feature = "waterui-preview-test-mode",
    feature = "waterui-mcp-mode"
)))]
fn main() {
    {% if ctx.cef_runtime_enabled() %}
    initialize_cef_runtime();
    {% endif %}
    let env = waterui::configure_environment!(waterui::env::Environment::new());
    let app = {{ ctx.crate_name_ident() }}::app(env);
    hydrolysis::run(app, hydrolysis_m3::Material3::defaults());
}
