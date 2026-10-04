use waterui::app::App;
use waterui::env::use_env;
use waterui::prelude::*;
use waterui::{include_web, js_api, preview};

/// Everything the page is allowed to reach: `#[js_api]` exposes each `async`
/// method as a `waterui.invoke("<name>", payload)` handler.
struct Api;

#[js_api]
impl Api {
    /// `await waterui.invoke("greet", { name })` in the page resolves to the
    /// returned string.
    async fn greet(&self, name: String) -> String {
        format!("Hello, {name} — from Rust")
    }
}

// The frontend in web/ is mounted on the waterui asset origin: `water run`
// serves it from the package manager's dev server in debug builds, and
// `water package`/`water run --release` stages the production build.
#[preview]
fn main() -> impl View {
    use_env(|env: Environment| {
        let resources = ResourceContext::from_environment(&env);
        include_web!(resources, "{{ ctx.web_arg() }}").serve(Api)
    })
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
