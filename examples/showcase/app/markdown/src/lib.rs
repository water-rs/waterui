//! Markdown example for WaterUI.
use waterui::app::App;
use waterui::prelude::*;

fn main() -> impl View {
    scroll(include_markdown!("example.md").padding())
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
