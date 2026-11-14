//! Settling must publish content a view owes that lands after real
//! wall-clock I/O — the remote-loading regression from issue #892 (`Photo`
//! fetches, `avatar` images in `water mcp` / `water preview`).

use std::time::Duration;

use waterui::task::spawn_local;
use waterui::{Binding, Environment, View};
use waterui_core::ContentLedger;
use waterui_testing::ui;

/// A view whose label lands after a wall-clock timer and which owes it
/// meanwhile — the shape of a `Photo` whose fetch completes on a reactor
/// thread.
struct DelayedLabel;

impl View for DelayedLabel {
    fn body(self, env: &Environment) -> impl View {
        let status = Binding::container(String::from("idle"));
        let owed = ContentLedger::owe::<Self>(env, "delayed label");
        spawn_local({
            let status = status.clone();
            async move {
                async_io::Timer::after(Duration::from_millis(100)).await;
                status.set(String::from("ready"));
                if let Some(owed) = owed {
                    owed.fulfil();
                }
            }
        })
        .detach();
        waterui::text!("{status}")
    }
}

#[test]
fn settle_publishes_owed_content_finished_after_wall_clock_io() {
    // Mounting settles the app. The load is parked on a wall-clock timer —
    // invisible to runnable-queue quiescence — so its result reaches the
    // tree only because settling waits on the content the view owes.
    let mut app = ui().mount(|| DelayedLabel);

    app.query().label_contains("ready").assert_exists();
}
