//! CEF subprocess entry point generated for {{ ctx.app_display_name }}.
//!
//! Chromium re-executes this application for its own renderer, GPU and utility
//! processes. Those must dispatch straight into CEF without starting WaterUI,
//! which is why they are a separate binary rather than a branch in `main`.

fn main() {
    std::process::exit(waterui_browser_cef::run_packaged_subprocess());
}
