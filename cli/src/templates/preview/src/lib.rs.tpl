use waterui::app::App;
use waterui::prelude::*;
use waterui_preview::Preview;

fn main() -> impl View {
    Preview::with_runtime_fingerprint("{{ ctx.preview_runtime_fingerprint() }}")
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
