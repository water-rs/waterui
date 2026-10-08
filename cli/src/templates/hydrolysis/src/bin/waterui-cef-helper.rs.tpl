//! CEF subprocess entry point generated for {{ ctx.app_display_name }}.
//!
//! Chromium re-executes this application for its own renderer, GPU and utility
//! processes. Those must dispatch straight into CEF without starting WaterUI,
//! which is why they are a separate binary rather than a branch in `main`.

// GUI subsystem like the application entry: each console-subsystem helper
// would open a console window when Chromium spawns it.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    std::process::exit(waterui_browser_cef::run_packaged_subprocess());
}
