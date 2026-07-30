//! Minimal CEF subprocess entry point for {{ ctx.app_display_name }}.

fn main() {
    std::process::exit(
        waterui_ffi::components::platform::browser_cef::waterui_cef_run_packaged_subprocess(),
    );
}
