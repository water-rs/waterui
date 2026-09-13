use waterui::app::App;
use waterui::prelude::*;
use waterui::{include_web, preview};

// The frontend in web/ is mounted on the waterui asset origin: `water run`
// serves it from the package manager's dev server in debug builds, and
// `water package`/`water run --release` stages the production build.
#[preview]
fn main() -> impl View {
    include_web!("{{ ctx.web_arg() }}")
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
