//! Native FFI companion crate for __APP_DISPLAY_NAME__.

use waterui::app::App;
use waterui::env::Environment;

fn app(env: Environment) -> App {
    __CRATE_NAME_IDENT__::app(env)
}

waterui_ffi::export!();
