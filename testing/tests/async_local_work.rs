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
    let mut app = ui().mount_offscreen(move || {
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

    app.semantic_mut()
        .query()
        .label_contains("ready")
        .assert_exists();
}

#[test]
fn settle_returns_while_a_live_producer_keeps_publishing() {
    let frames = Binding::container(0_u32);

    // Playback-style work: a task that outlives the settle and publishes a
    // frame every few milliseconds. Settling waits for parked work to publish,
    // not for such a producer to end — that would hold every interaction for
    // the whole wall-clock cap.
    let started = std::time::Instant::now();
    let mut app = ui().mount_offscreen(move || {
        waterui::text!("{frames}")
            .on_appear(|frames: waterui::State<Binding<u32>>| {
                spawn_local(async move {
                    loop {
                        async_io::Timer::after(Duration::from_millis(5)).await;
                        frames.set(frames.get() + 1);
                    }
                })
                .detach();
            })
            .state(&frames)
    });

    app.semantic_mut().settle();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "settle waited {:?} on a task that never finishes",
        started.elapsed()
    );
}
