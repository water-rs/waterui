//! Markdown example for WaterUI.
use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;

#[preview]
pub fn demo() -> impl View {
    scroll(include_markdown!("example.md").padding())
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
