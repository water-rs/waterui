//! CEF subprocess entry point generated for {{ ctx.app_display_name }}.
//!
//! Chromium re-executes this application for its own renderer, GPU and utility
//! processes. Those must dispatch straight into CEF without starting WaterUI,
//! which is why they are a separate binary rather than a branch in `main`.

// GUI subsystem like the application entry: each console-subsystem helper
// would open a console window when Chromium spawns it.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    // The `waterui-browser-cef` dependency lives in this manifest's per-OS
    // tables — the dispatch compiles only where an OS's table provides the
    // crate. On every other target the bin still compiles, but nothing
    // packages or spawns it, so running it there is a packaging bug.
    #[cfg({{ ctx.cef_helper_condition() }})]
    std::process::exit(waterui_browser_cef::run_packaged_subprocess());
    #[cfg(not({{ ctx.cef_helper_condition() }}))]
    unreachable!(
        "the CEF helper ran on a target whose manifest links no `waterui-browser-cef`; \
         only a CEF-linked build packages and spawns it"
    );
}
