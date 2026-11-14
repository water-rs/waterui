//! Snippets from `.claude/skills/waterui/references/project.md`, in file order.
//! Transcription conventions are documented in the crate README.
//!
//! project.md is mostly shell and TOML; it has five rust blocks.

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
// project.md § "## Project shape" — rust block 1/5
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
// project.md § "## Project shape" — rust block 2/5
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
// project.md § "## Permissions: declaring and requesting" — rust block 3/5
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
// project.md § "## Home-screen widgets" — rust block 4/5
// Needs `waterui`'s `widgets` feature (outside `all`); the harness Cargo.toml
// must enable it for this module to compile.
// ---------------------------------------------------------------------------
pub mod project_block_04 {
    use waterui::widgets::*;

    #[waterui::widget_intent(name = "Mark done")]
    pub struct MarkDone {
        id: u32,
    }

    impl WidgetIntent for MarkDone {
        fn perform(self) {
            waterui::log::info!("done {}", self.id);
        }
    }

    #[waterui::widget(name = "Tasks", description = "Open tasks", families = [systemSmall, systemMedium])]
    pub fn tasks(context: WidgetContext) -> impl WidgetContent {
        let title = Font::style(if context.family() == WidgetFamily::SystemSmall {
            FontStyle::Headline
        } else {
            FontStyle::Title
        });
        vstack((
            text("Tasks").font(title),
            intent(&MarkDone { id: 1 }, text("Mark done")),
        ))
        .alignment(HorizontalAlignment::Leading)
    }
}

// ---------------------------------------------------------------------------
// project.md § "## Logging and debugging" — rust block 5/5
// ---------------------------------------------------------------------------
pub fn project_block_05() {
    use waterui::log::{debug, info};

    let value = 42_i32;

    debug!(?value, "recomputed layout");
    info!("saved");
}
