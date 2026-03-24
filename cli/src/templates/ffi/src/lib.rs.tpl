//! Native FFI companion crate for {{ ctx.app_display_name }}.

use waterui::app::App;
use waterui::env::Environment;

fn app(env: Environment) -> App {
    {{ ctx.crate_name_ident() }}::app(env)
}

waterui_ffi::export!();
