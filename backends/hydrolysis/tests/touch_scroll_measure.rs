//! water-rs/waterui#2014: a touch drag or fling on a lazy `List` must keep
//! scrolling while the list measures rows whose heights differ from its
//! estimate. Each newly measured row changes the list's `total_extent`, which
//! rebinds the list's `ScrollHandle` and advances its generation; the gesture
//! still owns the scroll view and its writes must keep landing.
//!
//! Touch-scroll claim, slop and fling machinery is the backend's own, so the
//! tests stay here under water-rs/waterui#1673, mounted on `HeadlessRuntime`
//! under `TestStyle`.

mod support;

use accesskit::Role;
use support::{mount_rendered, nodes};
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui::{View, ViewExt};
use waterui_core::Environment;

const VIEWPORT: (u32, u32) = (390, 844);
const ROW_COUNT: usize = 120;
/// Android's density-1 touch slop: the claim applies only the drag's excess
/// over it, so the offset trails the finger by exactly this much.
const TOUCH_SLOP: f64 = 10.0;
const DRAG_X: f32 = 195.0;
const DRAG_START_Y: f32 = 760.0;
/// Each drag sample moves the finger this far up, one sample per frame.
const DRAG_STEP: f32 = 50.0;
const DRAG_STEPS: u32 = 12;
/// The fling's lead-in drag: two fast samples a frame apart.
const FLING_STEPS: u32 = 2;
const FLING_DISTANCE: f32 = 240.0;
/// Frames sampled after release; a ~7000 pt/s Android spline fling runs for
/// well over a second, far past this window.
const FLING_FRAMES: usize = 30;
/// One virtual frame.
const FRAME: std::time::Duration = std::time::Duration::from_millis(16);

/// Rows of one to four text lines: every row measures taller or shorter than
/// the list's single seed estimate, so revealing a row changes the extent.
fn uneven_list() -> impl View {
    List::content(
        (0..ROW_COUNT)
            .map(|index| {
                move || {
                    let lines = (0..=index % 4)
                        .map(|line| format!("row {index} line {line}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    ListItem::new(text(lines))
                }
            })
            .collect::<Vec<_>>(),
    )
    .a11y_label("uneven")
}

fn mount() -> support::RenderedApp {
    let mut app = mount_rendered(Environment::new(), uneven_list, VIEWPORT.0, VIEWPORT.1);
    app.set_touch_scroll_config(hydrolysis::TouchScrollConfig::android_default());
    app.settle();
    app
}

fn list_offset(app: &mut support::RenderedApp) -> f64 {
    nodes(app.tree())
        .find(|(_, node)| node.role() == Role::List && node.label() == Some("uneven"))
        .expect("the rendered list emits a node")
        .1
        .scroll_y()
        .expect("the rendered list reports its offset")
}

/// The offset after the first `steps` samples of the same drag, read before
/// the release is processed: one fresh mount per prefix, so the samples are
/// the per-frame offsets of a single gesture.
#[expect(
    clippy::cast_precision_loss,
    reason = "the drag runs at most a handful of steps"
)]
fn offset_after_drag_prefix(steps: u32) -> f64 {
    let mut app = mount();
    // Nothing has scrolled the fresh mount: its offset is the initial
    // value, exactly zero.
    assert_eq!(
        list_offset(&mut app).to_bits(),
        0.0_f64.to_bits(),
        "the list starts at the top"
    );
    let distance = DRAG_STEP * steps as f32;
    app.queue_drag_from_to_with(
        DRAG_X,
        DRAG_START_Y,
        DRAG_X,
        DRAG_START_Y - distance,
        steps,
        true,
        hydrolysis::PointerKind::Touch,
    );
    list_offset(&mut app)
}

#[test]
#[expect(
    clippy::cast_precision_loss,
    reason = "the drag runs at most a handful of steps"
)]
fn a_touch_drag_follows_the_finger_while_rows_measure() {
    let observed: Vec<(f64, f64)> = (1..=DRAG_STEPS)
        .map(|steps| {
            let expected = f64::from(DRAG_STEP * steps as f32) - TOUCH_SLOP;
            (expected, offset_after_drag_prefix(steps))
        })
        .collect();
    assert!(
        observed
            .iter()
            .all(|(expected, offset)| (expected - offset).abs() < 0.5),
        "the offset must follow the finger frame by frame, (expected, observed): {observed:?}"
    );
}

#[test]
fn a_touch_fling_keeps_advancing_while_rows_measure() {
    let mut app = mount();
    // Nothing has scrolled the fresh mount: its offset is the initial
    // value, exactly zero.
    assert_eq!(
        list_offset(&mut app).to_bits(),
        0.0_f64.to_bits(),
        "the list starts at the top"
    );
    app.queue_drag_from_to_with(
        DRAG_X,
        DRAG_START_Y,
        DRAG_X,
        DRAG_START_Y - FLING_DISTANCE,
        FLING_STEPS,
        true,
        hydrolysis::PointerKind::Touch,
    );
    let released_at = list_offset(&mut app);
    let mut offsets = vec![released_at];
    for _ in 0..FLING_FRAMES {
        app.pump_for(FRAME);
        offsets.push(list_offset(&mut app));
    }
    assert!(
        (released_at - (f64::from(FLING_DISTANCE) - TOUCH_SLOP)).abs() < 0.5,
        "the lead-in drag must follow the finger before the fling: {offsets:?}"
    );
    // Frame 0 processes the release, which starts the fling at the release
    // offset; every later frame must move further down the content.
    assert!(
        offsets[1..].windows(2).all(|pair| pair[1] > pair[0]),
        "the fling must keep advancing for {FLING_FRAMES} frames ({} ms), offsets: {offsets:?}",
        FRAME.as_millis() * u128::try_from(FLING_FRAMES).expect("small")
    );
}
