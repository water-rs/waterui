//! Native companion crate for {{ ctx.app_display_name }}.

use waterui::app::{App, Windows};
use waterui::env::Environment;

fn app(env: Environment) -> App<impl Windows> {
    {{ ctx.crate_name_ident() }}::app(env)
}

#[cfg(target_vendor = "apple")]
waterui_apple::export_app!(app);
