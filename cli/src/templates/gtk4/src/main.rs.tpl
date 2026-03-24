//! GTK entry point for {{ ctx.app_display_name }}.

use waterui_gtk::{Environment, GtkApp, init_main_thread_executors};

fn main() {
    init_main_thread_executors();
    let app = {{ ctx.crate_name_ident() }}::app(waterui::configure_environment!(Environment::new()));
    GtkApp::default().run_app(app);
}
