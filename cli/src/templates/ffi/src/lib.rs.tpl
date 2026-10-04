//! Native companion crate for {{ ctx.app_display_name }}.

use waterui::app::App;
use waterui::env::Environment;

// Apple owns its native entry and mount; other targets use the existing FFI entry.
fn app(env: Environment) -> App {
    {{ ctx.crate_name_ident() }}::app(env)
}

#[cfg(not(target_vendor = "apple"))]
waterui_ffi::export!();
{% if ctx.apple_backend_selected %}
#[cfg(target_vendor = "apple")]
waterui_apple::export_app!(app);
{% endif %}
