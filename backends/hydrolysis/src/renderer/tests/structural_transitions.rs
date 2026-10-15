//! layout-spec.md §4.5 structural transitions at `Dynamic` sites
//! (`when`/`watch`): the swapped-out instance leaves the accessibility tree on
//! the frame the condition changes, keeps painting — non-interactive — in the
//! slot it was placed at until its own removal clock completes, and only then
//! releases that slot; an instance swapped in allocates its own slot at once
//! and runs an independent insertion. The declaration is claimed from the
//! built instance, so it may sit beneath other modifiers or behind type
//! erasure, or over the conditional itself.
//!
//! Geometry is read from the accessibility tree; mid-transition paint from
//! captured frames. Every frame is pumped at an explicit instant.

use core::time::Duration;
use std::time::Instant;

use accesskit::{Rect, Role};
use nami::Binding;
use waterui::ViewExt as _;
use waterui::animation::Animation;
use waterui::graphics::color::Srgb;
use waterui::shape::RoundedRectangle;
use waterui::widget::condition::when;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::transition::{PropertyTransition, Transition};
use waterui_core::{AnyView, View};
use waterui_layout::padding::EdgeInsets;
use waterui_layout::stack::vstack;
use waterui_shape::ShapeExt as _;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::runner::HeadlessSnapshot;

const RED: Srgb = Srgb::new_u8(220, 40, 40);
const GREEN: Srgb = Srgb::new_u8(40, 180, 70);
const BLUE: Srgb = Srgb::new_u8(40, 70, 220);
/// Every transition here runs a linear one-second clock.
const CLOCK: Duration = Duration::from_millis(1_000);

/// A 120x40 swatch whose accessibility node carries `label`.
fn swatch(label: &'static str, color: Srgb) -> impl View {
    RoundedRectangle::new(0.0)
        .fill(color)
        .a11y_label(label)
        .size(120.0, 40.0)
}

/// `swatch` declaring `transition` on its root.
fn transitioned(label: &'static str, color: Srgb, transition: impl Transition) -> impl View {
    swatch(label, color)
        .transition(transition)
        .animation(Animation::linear(CLOCK))
}

fn runtime(view: impl Fn() -> AnyView + 'static) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(view),
        400,
        300,
        MinimalTestTheme::default(),
    )
}

/// The bounds the current accessibility tree carries for `label`, `None`
/// when the node is absent.
fn bounds(runtime: &mut HeadlessRuntime, label: &str) -> Option<Rect> {
    runtime.accessibility_tree().and_then(|update| {
        update.nodes.iter().find_map(|(_, node)| {
            (node.role() == Role::Image && node.label() == Some(label))
                .then(|| node.bounds())
                .flatten()
        })
    })
}

fn frame(runtime: &mut HeadlessRuntime, at: Instant) -> HeadlessSnapshot {
    runtime
        .pump_at(true, at)
        .snapshot
        .expect("a pumped frame must be capturable")
}

/// The pixel at the centre of `rect`.
fn centre_pixel(snapshot: &HeadlessSnapshot, rect: Rect) -> [u8; 4] {
    let x = crate::num_cast::f64_as_u32(f64::midpoint(rect.x0, rect.x1));
    let y = crate::num_cast::f64_as_u32(f64::midpoint(rect.y0, rect.y1));
    let index = usize::try_from((y * snapshot.width + x) * 4).expect("a pixel index fits usize");
    snapshot.rgba8[index..index + 4]
        .try_into()
        .expect("four channels per pixel")
}

fn near(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.5
}

/// Writes `snapshot` under the test artifacts for visual review.
fn save(snapshot: &HeadlessSnapshot, case: &str, name: &str) {
    let path = crate::testing::artifact_snapshot_path("hydrolysis", case, name);
    std::fs::create_dir_all(path.parent().expect("the case directory"))
        .expect("the capture directory must be creatable");
    image::RgbaImage::from_raw(snapshot.width, snapshot.height, snapshot.rgba8.clone())
        .expect("snapshot dimensions must match the rgba buffer")
        .save(&path)
        .expect("the frame must be writable");
}

/// A `when` branch that turns false: its instance leaves the accessibility
/// tree on the event frame, fades in the slot it was placed at while the
/// sibling below stays put, and releases the slot when the clock completes.
#[test]
fn a_when_removal_leaves_semantics_at_once_and_holds_its_slot() {
    let show = Binding::container(true);
    let show_for_view = show.clone();
    let mut runtime = runtime(move || {
        AnyView::new(vstack((
            when(show_for_view.clone(), || {
                transitioned("leaving", RED, PropertyTransition::opacity())
            }),
            swatch("sibling", BLUE),
        )))
    });
    let start = Instant::now();
    let rest = frame(&mut runtime, start);
    let leaving = bounds(&mut runtime, "leaving").expect("the branch renders at rest");
    let sibling = bounds(&mut runtime, "sibling").expect("the sibling renders at rest");

    show.set(false);
    let _ = frame(&mut runtime, start + Duration::from_millis(16));
    assert!(
        bounds(&mut runtime, "leaving").is_none(),
        "the removed instance leaves the accessibility tree on the event frame"
    );
    let mid = frame(&mut runtime, start + Duration::from_millis(516));
    let held = bounds(&mut runtime, "sibling").expect("the sibling renders mid-removal");
    assert!(
        near(held.y0, sibling.y0),
        "the ghost holds its slot, so the sibling stays put: rest {sibling:?}, mid {held:?}"
    );
    let ghost = centre_pixel(&mid, leaving);
    assert!(
        ghost != centre_pixel(&rest, leaving),
        "halfway through the removal the ghost is fading, not at rest"
    );

    let _ = frame(&mut runtime, start + Duration::from_millis(1_100));
    let _ = frame(&mut runtime, start + Duration::from_millis(1_116));
    let released = bounds(&mut runtime, "sibling").expect("the sibling renders after");
    assert!(
        near(released.y0, leaving.y0),
        "once the removal completes the slot is released and the sibling moves up \
         into it: leaving {leaving:?}, sibling {released:?}"
    );
}

/// A `when` branch that turns true: the new instance is in the accessibility
/// tree and holds its slot from the event frame, and fades in on its own
/// clock.
#[test]
fn a_when_insertion_allocates_its_slot_at_once_and_fades_in() {
    let show = Binding::container(false);
    let show_for_view = show.clone();
    let mut runtime = runtime(move || {
        AnyView::new(vstack((
            when(show_for_view.clone(), || {
                transitioned("entering", RED, PropertyTransition::opacity())
            }),
            swatch("sibling", BLUE),
        )))
    });
    let start = Instant::now();
    let _ = frame(&mut runtime, start);
    let sibling = bounds(&mut runtime, "sibling").expect("the sibling renders at rest");

    show.set(true);
    let _ = frame(&mut runtime, start + Duration::from_millis(16));
    let entering = bounds(&mut runtime, "entering").expect("the inserted instance is in the tree");
    let pushed = bounds(&mut runtime, "sibling").expect("the sibling renders");
    assert!(
        near(entering.y0, sibling.y0) && pushed.y0 >= entering.y1,
        "the insertion allocates its slot on the event frame: entering {entering:?}, \
         sibling {pushed:?}"
    );
    let mid = frame(&mut runtime, start + Duration::from_millis(516));
    let settled = frame(&mut runtime, start + Duration::from_millis(1_100));
    assert!(
        centre_pixel(&mid, entering) != centre_pixel(&settled, entering),
        "halfway through the insertion the instance is still fading in"
    );
}

/// A transition declared over the conditional itself applies to every
/// instance it swaps: the outgoing branch's ghost keeps its slot ahead of the
/// incoming branch, which takes a new slot after it; when the removal clock
/// completes the incoming branch moves up into the released slot.
#[test]
fn a_transition_over_the_conditional_swaps_with_a_ghost_beside_the_new_branch() {
    let first = Binding::container(true);
    let first_for_view = first.clone();
    let mut runtime = runtime(move || {
        AnyView::new(vstack((
            when(first_for_view.clone(), || swatch("first", RED))
                .otherwise(|| swatch("second", GREEN))
                .transition(PropertyTransition::opacity())
                .animation(Animation::linear(CLOCK)),
            swatch("sibling", BLUE),
        )))
    });
    let start = Instant::now();
    let _ = frame(&mut runtime, start);
    let slot = bounds(&mut runtime, "first").expect("the first branch renders at rest");

    first.set(false);
    let _ = frame(&mut runtime, start + Duration::from_millis(16));
    assert!(
        bounds(&mut runtime, "first").is_none(),
        "the swapped-out branch leaves the accessibility tree at once"
    );
    let _ = frame(&mut runtime, start + Duration::from_millis(516));
    let second = bounds(&mut runtime, "second").expect("the new branch is in the tree");
    assert!(
        second.y0 >= slot.y1,
        "the ghost keeps the old slot and the new branch takes the next one: slot \
         {slot:?}, second {second:?}"
    );

    let _ = frame(&mut runtime, start + Duration::from_millis(1_100));
    let _ = frame(&mut runtime, start + Duration::from_millis(1_116));
    let settled = bounds(&mut runtime, "second").expect("the new branch renders");
    assert!(
        near(settled.y0, slot.y0),
        "once the removal completes the new branch moves up into the released slot: \
         slot {slot:?}, second {settled:?}"
    );
}

/// A declaration beneath other modifiers and behind type erasure is claimed
/// by the conditional that swaps it: the padded ghost keeps its whole padded
/// slot until its clock completes.
#[test]
fn a_declaration_beneath_modifiers_and_erasure_is_owned_by_the_conditional() {
    let show = Binding::container(true);
    let show_for_view = show.clone();
    let mut runtime = runtime(move || {
        AnyView::new(vstack((
            when(show_for_view.clone(), || {
                AnyView::new(transitioned("leaving", RED, PropertyTransition::opacity()))
                    .padding_with(EdgeInsets::all(8.0))
            }),
            swatch("sibling", BLUE),
        )))
    });
    let start = Instant::now();
    let _ = frame(&mut runtime, start);
    let leaving = bounds(&mut runtime, "leaving").expect("the branch renders at rest");
    let sibling = bounds(&mut runtime, "sibling").expect("the sibling renders at rest");

    show.set(false);
    let _ = frame(&mut runtime, start + Duration::from_millis(16));
    assert!(bounds(&mut runtime, "leaving").is_none());
    let _ = frame(&mut runtime, start + Duration::from_millis(516));
    let held = bounds(&mut runtime, "sibling").expect("the sibling renders mid-removal");
    assert!(
        near(held.y0, sibling.y0),
        "the padded ghost holds its padded slot: rest {sibling:?}, mid {held:?}"
    );
    let _ = frame(&mut runtime, start + Duration::from_millis(1_100));
    let _ = frame(&mut runtime, start + Duration::from_millis(1_116));
    let released = bounds(&mut runtime, "sibling").expect("the sibling renders after");
    assert!(
        near(released.y0, leaving.y0 - 8.0),
        "the released slot includes the padding: leaving {leaving:?}, sibling {released:?}"
    );
}

/// A blur transition is realized as a property: halfway through the removal
/// the ghost draws through a Gaussian blur filter layer inside its held slot
/// — its edge softens toward the background — and the slot is released when
/// the clock completes. The mid-flight frame lands under
/// `<artifacts>/hydrolysis/structural_transition_blur/mid-removal.png`.
#[test]
fn a_blur_removal_draws_the_ghost_blurred_mid_flight() {
    let show = Binding::container(true);
    let show_for_view = show.clone();
    let mut runtime = runtime(move || {
        AnyView::new(vstack((
            when(show_for_view.clone(), || {
                transitioned("leaving", RED, PropertyTransition::blur(12.0))
            }),
            swatch("sibling", BLUE),
        )))
    });
    let start = Instant::now();
    let rest = frame(&mut runtime, start);
    let leaving = bounds(&mut runtime, "leaving").expect("the branch renders at rest");
    let sibling = bounds(&mut runtime, "sibling").expect("the sibling renders at rest");

    show.set(false);
    let _ = frame(&mut runtime, start + Duration::from_millis(16));
    let mid = frame(&mut runtime, start + Duration::from_millis(516));
    save(&mid, "structural_transition_blur", "mid-removal");
    // One pixel inside the swatch's trailing edge: solid at rest, blended
    // with the background once the blur spreads the edge.
    let middle = f64::midpoint(leaving.y0, leaving.y1);
    let edge = Rect::new(leaving.x1 - 2.0, middle - 1.0, leaving.x1, middle + 1.0);
    let centre = Rect::new(
        f64::midpoint(leaving.x0, leaving.x1) - 1.0,
        middle - 1.0,
        f64::midpoint(leaving.x0, leaving.x1) + 1.0,
        middle + 1.0,
    );
    assert!(
        centre_pixel(&mid, edge) != centre_pixel(&rest, edge),
        "the blurred ghost's edge softens mid-flight"
    );
    assert!(
        centre_pixel(&mid, centre) != centre_pixel(&mid, edge),
        "the ghost is blurred, not faded: its centre stays denser than its edge"
    );
    let held = bounds(&mut runtime, "sibling").expect("the sibling renders mid-removal");
    assert!(
        near(held.y0, sibling.y0),
        "the blurred ghost holds its slot: rest {sibling:?}, mid {held:?}"
    );
    let _ = frame(&mut runtime, start + Duration::from_millis(1_100));
    let _ = frame(&mut runtime, start + Duration::from_millis(1_116));
    let released = bounds(&mut runtime, "sibling").expect("the sibling renders after");
    assert!(
        near(released.y0, leaving.y0),
        "after the removal the slot is released: leaving {leaving:?}, sibling {released:?}"
    );
}
