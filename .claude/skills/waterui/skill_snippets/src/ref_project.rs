//! Snippets from `.claude/skills/waterui/references/project.md`, in file order.
//! Transcription conventions are documented in the crate README.
//!
//! project.md is mostly shell and TOML; it has four rust blocks.

use waterui::prelude::*;

/// Glue: the root view `App::new(root_view, env)` refers to.
fn root_view() -> impl View {
    text("root")
}

/// Glue: the app-wide state the second block clones into the builder.
#[derive(Clone)]
pub struct AppState;

impl AppState {
    fn new() -> Self {
        Self
    }
}

fn content(_state: AppState) -> impl View {
    text("content")
}

// ---------------------------------------------------------------------------
// project.md § "## Project shape" — rust block 1/4
// ---------------------------------------------------------------------------
pub mod project_block_01 {
    use super::root_view;

    use waterui::app::App;
    use waterui::prelude::*;

    pub fn app(env: Environment) -> App {
        App::new(root_view, env)
    }
}

// ---------------------------------------------------------------------------
// project.md § "## Project shape" — rust block 2/4
// ---------------------------------------------------------------------------
pub mod project_block_02 {
    use super::{AppState, content};
    use waterui::app::App;
    use waterui::prelude::*;

    pub fn app(env: Environment) -> App {
        let state = AppState::new();
        App::new(move || content(state.clone()), env)
    }
}

// ---------------------------------------------------------------------------
// project.md § "## Permissions: declaring and requesting" — rust block 3/4
// ---------------------------------------------------------------------------
pub fn project_block_03() -> impl View {
    let granted = Binding::bool(false);

    use waterkit_permission::{Permission, PermissionStatus, check, request};

    button("Enable microphone")
        .action_async(|State(granted): State<Binding<bool>>| async move {
            let status = check(Permission::Microphone).await; // infallible
            let status = if matches!(status, PermissionStatus::Granted) {
                status
            } else {
                match request(Permission::Microphone).await {
                    // fallible: Result
                    Ok(s) => s,
                    Err(_) => return,
                }
            };
            granted.set(matches!(status, PermissionStatus::Granted));
        })
        .state(&granted)
}

// ---------------------------------------------------------------------------
// project.md § "## Logging and debugging" — rust block 4/4
// ---------------------------------------------------------------------------
pub fn project_block_04() {
    let value = 42_i32;

    waterui::log::debug!(?value, "recomputed layout");
    waterui::log::info!("saved");
}

// ---------------------------------------------------------------------------
// project.md § "## Embedded targets (Dew)" (prose):
// "Headless snapshot: `waterui_dew::render_view_png(builder, env, w, h)`."
//
// NOT COMPILABLE BY DESIGN here: `waterui-dew` is a separate backend crate an
// app does not depend on, and adding it would say nothing about the `waterui`
// API surface this crate gates. Recorded, not transcribed.
// ---------------------------------------------------------------------------
