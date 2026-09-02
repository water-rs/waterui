//! Markdown example for WaterUI.
use waterui::app::App;
use waterui::env::use_env;
use waterui::metadata::Metadata;
use waterui::prelude::*;
use waterui::preview;

#[preview]
pub fn demo() -> impl View {
    // Installing the Mermaid realization is the application's job — `waterui`
    // has no dependency on `waterui-mermaid`, which is what lets a build that
    // does not want diagrams render the fence as plain code. It goes here
    // rather than in `app` because `water preview` renders this function
    // directly, so one site serves both entry points.
    use_env(|mut env: Environment| {
        waterui_mermaid::install(&mut env);
        Metadata::new(scroll(include_markdown!("example.md").padding()), env)
    })
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
