//! water-rs/waterui#1952 — an `.opacity(binding)` write after the first frame
//! must reach the flushed layer's alpha on the next pump, with no view-tree
//! rebuild. The `Opacity` leaf re-samples its `Computed<f32>` each flush and
//! the signal's mark arms the frame; a headless runtime that kept the first
//! frame's alpha left the binding disconnected.

use std::time::{Duration, Instant};

use nami::Binding;
use waterui::ViewExt as _;
use waterui::graphics::color::Srgb;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;

const WINDOW_WIDTH: u32 = 320;
const WINDOW_HEIGHT: u32 = 240;
const RED: [u8; 3] = [0xE6, 0x26, 0x26];

fn runtime(builder: AnyViewBuilder<AnyView>) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

/// Count of pixels near `rgb` over the whole snapshot.
fn count_rgb(rgba8: &[u8], rgb: [u8; 3], tolerance: u8) -> usize {
    rgba8
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| {
            px[..3]
                .iter()
                .zip(rgb)
                .all(|(channel, target)| channel.abs_diff(target) <= tolerance)
        })
        .count()
}

#[test]
fn an_opacity_binding_change_reaches_the_flushed_alpha() {
    let alpha = Binding::container(1.0f32);
    let view_alpha = alpha.clone();
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(Srgb::from(RED).opacity(view_alpha.clone()))
    });
    let mut runtime = runtime(builder);
    let mut at = Instant::now();

    let painted = runtime
        .pump_at(true, at)
        .into_snapshot()
        .expect("the mount frame captures a snapshot");
    assert!(
        count_rgb(painted.rgba8(), RED, 2) > 0,
        "the fully opaque color must paint on the first frame"
    );

    alpha.set(0.0);
    at += Duration::from_millis(16);
    let result = runtime.pump_at(true, at);
    assert!(
        !result.rebuilt(),
        "a binding write must not rebuild the view tree"
    );
    let faded = result
        .snapshot()
        .expect("the post-write frame captures a snapshot");
    assert_eq!(
        count_rgb(faded.rgba8(), RED, 2),
        0,
        "opacity 0.0 must reach the flushed layer's alpha on the next frame"
    );

    alpha.set(0.5);
    at += Duration::from_millis(16);
    let half = runtime
        .pump_at(true, at)
        .into_snapshot()
        .expect("the restore frame captures a snapshot");
    assert_eq!(
        count_rgb(half.rgba8(), RED, 2),
        0,
        "alpha 0.5 blends over the window background — the pure source stays absent"
    );
    assert_ne!(
        half.rgba8(),
        faded.rgba8(),
        "alpha 0.5 must repaint every pixel the transparent frame left at the background"
    );
}
