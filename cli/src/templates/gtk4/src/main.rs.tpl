//! GTK entry point for __APP_DISPLAY_NAME__.

use waterui_gtk::{Environment, GtkApp};

fn main() {
    let app = __CRATE_NAME_IDENT__::app(Environment::new());
    GtkApp::default().run_app(app);
}
