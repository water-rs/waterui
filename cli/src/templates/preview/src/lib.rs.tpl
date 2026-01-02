use waterui::app::App;
use waterui::prelude::*;
use waterui_preview::Preview;

fn main() -> impl View {
    Preview::new()
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
