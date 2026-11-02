//! Native companion crate for {{ ctx.app_display_name }}.

use waterui::app::App;
use waterui::env::Environment;

fn app(env: Environment) -> App {
    {{ ctx.crate_name_ident() }}::app(env)
}

#[cfg(target_vendor = "apple")]
waterui_apple::export_app!(app);
