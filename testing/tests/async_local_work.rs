//! Settling must publish `spawn_local` work that completes after real
//! wall-clock I/O — the remote-loading regression from issue #892 (`Photo`
//! fetches, `avatar` images in `water mcp` / `water preview`).

use std::time::Duration;

use waterui::task::spawn_local;
use waterui::{Binding, ViewExt as _};
use waterui_testing::ui;

#[test]
fn settle_publishes_task_finished_after_wall_clock_io() {
    let status = Binding::container(String::from("idle"));

    // Mounting settles the app. The `on_appear` task is parked on a
    // wall-clock timer — invisible to runnable-queue quiescence — so its
    // result reaches the tree only if settling gives it real time.
    let mut app = ui().mount(move || {
        waterui::text!("{status}")
            .on_appear(|status: waterui::State<Binding<String>>| {
                spawn_local(async move {
                    async_io::Timer::after(Duration::from_millis(100)).await;
                    status.set(String::from("ready"));
                })
                .detach();
            })
            .state(&status)
    });

    app.query().label_contains("ready").assert_exists();
}
