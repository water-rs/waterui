//! Hydrolysis entry point for __APP_DISPLAY_NAME__.

use hydrolysis::run;
use waterui::env::Environment;

fn main() {
    let app = __CRATE_NAME_IDENT__::app(waterui::configure_environment!(Environment::new()));
    run(app);
}
