use waterui::app::App;
use waterui::prelude::*;
use waterui_preview::{Preview, init_tracing_from_env};

fn main() -> impl View {
    init_tracing_from_env();
    Preview::with_runtime_fingerprint("{{ ctx.preview_runtime_fingerprint() }}")
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
