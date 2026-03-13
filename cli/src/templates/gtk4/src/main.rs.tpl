//! GTK entry point for __APP_DISPLAY_NAME__.

use waterui_gtk::{Environment, GtkApp, init_main_thread_executors};

fn main() {
    init_main_thread_executors();
    let app = __CRATE_NAME_IDENT__::app(waterui::configure_environment!(Environment::new()));
    GtkApp::default().run_app(app);
}
