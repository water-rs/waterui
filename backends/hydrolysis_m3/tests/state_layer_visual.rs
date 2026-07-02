//! Visual acceptance for MD3 interaction state layers on the retained render
//! path: press ripple growth, hover tint, and ripple survival across a
//! same-frame structural patch (a button whose action swaps a `watch` subtree,
//! the chart-demo mode-switch scenario).
//!
//! The PNG-producing tests are ignored by default and reviewed by eye.

use core::time::Duration;
use hydrolysis_m3::install;
use waterui::component::{text, vstack};
use waterui::reactive::binding;
use waterui::AnyView;
use waterui_core::dynamic::watch;
use waterui_controls::button;
use waterui_testing::{OffscreenApp, ui};

fn press_center(app: &mut OffscreenApp, label: &str) -> (f32, f32) {
    assert!(
        app.query()
            .label(label)
            .wait_for_existence(Duration::from_secs(3)),
        "button must mount"
    );
    let bounds = app.query().label(label).single().bounds();
    (
        bounds.x() + bounds.width() / 2.0,
        bounds.y() + bounds.height() / 2.0,
    )
}

fn save(app: &mut OffscreenApp, stage: &str) {
    let _ = app.capture_snapshot("material3-preview", "state-layers", stage);
}

#[test]
#[ignore = "writes visual acceptance PNGs for direct image review"]
fn plain_button_press_shows_growing_ripple() {
    let mut app = ui()
        .viewport(360, 200)
        .theme(install)
        .mount(|| button("Press Me"));
    let (cx, cy) = press_center(&mut app, "Press Me");
    save(&mut app, "press-before");
    // Queue the press without the semantic settle so the captures land inside
    // the 225ms ripple growth instead of pumping straight past it.
    app.queue_pointer_down(cx - 40.0, cy + 10.0);
    save(&mut app, "press-0ms");
    std::thread::sleep(Duration::from_millis(60));
    save(&mut app, "press-60ms");
    std::thread::sleep(Duration::from_millis(80));
    save(&mut app, "press-140ms");
    std::thread::sleep(Duration::from_millis(120));
    save(&mut app, "press-260ms");
}

#[test]
#[ignore = "writes visual acceptance PNGs for direct image review"]
fn released_ripple_fades_at_full_size_without_shrinking() {
    // Regression stage for the reverse-playback bug: after release the wave
    // must hold its expanded, centered shape and only lose opacity — the
    // captures after pointer-up must never show a smaller circle drifting
    // back toward the press point.
    let mut app = ui()
        .viewport(360, 200)
        .theme(install)
        .mount(|| button("Release Me"));
    let (cx, cy) = press_center(&mut app, "Release Me");
    // Press off-center so a shrinking ripple would visibly drift.
    let (px, py) = (cx - 40.0, cy + 10.0);
    app.queue_pointer_down(px, py);
    let _ = app.snapshot();
    // Hold until the growth (225ms) has completed, then release: the fade-out
    // applies immediately (the minimum press duration equals the grow time).
    std::thread::sleep(Duration::from_millis(250));
    save(&mut app, "hold-250ms-full-size");
    app.queue_pointer_up(px, py);
    let _ = app.snapshot();
    std::thread::sleep(Duration::from_millis(50));
    save(&mut app, "release-50ms-fading");
    std::thread::sleep(Duration::from_millis(60));
    save(&mut app, "release-110ms-fainter");
    std::thread::sleep(Duration::from_millis(120));
    save(&mut app, "release-230ms-gone");
}

#[test]
#[ignore = "writes visual acceptance PNGs for direct image review"]
fn rapid_represses_overlap_independent_waves() {
    // mdui multi-wave behavior: a quick tap's wave keeps fading at full size
    // while a second press spawns a fresh wave growing from its own point —
    // the overlap capture must show both at once.
    let mut app = ui()
        .viewport(360, 200)
        .theme(install)
        .mount(|| button("Tap Tap"));
    let (cx, cy) = press_center(&mut app, "Tap Tap");
    // Stay inside the button bounds; opposite corners make the two waves'
    // distinct origins obvious.
    let (p1x, p1y) = (cx - 25.0, cy + 8.0);
    let (p2x, p2y) = (cx + 25.0, cy - 8.0);
    app.queue_pointer_down(p1x, p1y);
    let _ = app.snapshot();
    app.queue_pointer_up(p1x, p1y);
    let _ = app.snapshot();
    // The first wave finishes growing (225ms) and starts its deferred fade.
    std::thread::sleep(Duration::from_millis(240));
    app.queue_pointer_down(p2x, p2y);
    let _ = app.snapshot();
    std::thread::sleep(Duration::from_millis(60));
    save(&mut app, "repress-overlap-60ms");
    std::thread::sleep(Duration::from_millis(80));
    save(&mut app, "repress-140ms-second-wave");
}

#[test]
#[ignore = "writes visual acceptance PNGs for direct image review"]
fn plain_button_hover_shows_state_layer() {
    let mut app = ui()
        .viewport(360, 200)
        .theme(install)
        .mount(|| button("Hover Me"));
    let _ = press_center(&mut app, "Hover Me");
    save(&mut app, "hover-before");
    let _ = app.query().label("Hover Me").hover();
    std::thread::sleep(Duration::from_millis(120));
    save(&mut app, "hover-120ms");
}

#[test]
#[ignore = "writes visual acceptance PNGs for direct image review"]
fn ripple_survives_same_frame_structural_patch() {
    // The chart-demo scenario: the button's action (dispatched on pointer
    // down) flips a signal that a `watch` subtree rebuilds from in the same
    // refresh frame. The press ripple on the button must keep animating.
    let mode_for_view = binding(false);
    let mut app = ui().viewport(360, 240).theme(install).mount(move || {
        let mode_for_action = mode_for_view.clone();
        let swapped = watch(mode_for_view.clone(), |mode| {
            if mode {
                AnyView::new(text("Swapped content"))
            } else {
                AnyView::new(text("Initial content"))
            }
        });
        vstack((
            button("Swap").action(move || {
                let next = !mode_for_action.get();
                mode_for_action.set(next);
            }),
            swapped,
        ))
    });
    let (cx, cy) = press_center(&mut app, "Swap");
    assert!(app.semantic_mut().pointer_down_at(cx, cy), "press must hit");
    std::thread::sleep(Duration::from_millis(80));
    save(&mut app, "patch-press-80ms");
    std::thread::sleep(Duration::from_millis(140));
    save(&mut app, "patch-press-220ms");
}
