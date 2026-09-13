//! Markdown example for WaterUI.
use waterui::app::App;
use waterui::env::use_env;
use waterui::metadata::Metadata;
use waterui::prelude::*;
use waterui::preview;

#[preview]
pub fn demo() -> impl View {
    use_env(|env: Environment| {
        Metadata::new(scroll(include_markdown!("example.md").padding()), env)
    })
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
