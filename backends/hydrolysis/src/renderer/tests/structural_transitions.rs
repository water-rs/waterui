//! layout-spec.md §4.5 structural transitions at `Dynamic` sites
//! (`when`/`watch`): the swapped-out instance leaves the accessibility tree on
//! the frame the condition changes and keeps painting — non-interactive — at
//! the frame it left at until its own removal clock completes, overlapping
//! the conditional's one slot, which the live content alone sizes whatever
//! layout encloses it; an instance swapped in takes that slot at once and
//! runs an independent insertion. The declaration is claimed from the root
//! of the built instance, so it may sit behind type erasure or over the
//! conditional itself, but never beneath another modifier.
//!
//! Geometry is read from the accessibility tree; mid-transition paint from
//! captured frames. Every frame is pumped at an explicit instant.

use core::time::Duration;
use std::time::Instant;

use accesskit::{Rect, Role};
use nami::Binding;
use waterui::ViewExt as _;
use waterui::animation::Animation;
use waterui::graphics::Color;
use waterui::graphics::color::Srgb;
use waterui::shape::RoundedRectangle;
use waterui::widget::condition::when;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::layout::HorizontalAlignment;
use waterui_core::transition::{PropertyTransition, Transition};
use waterui_core::{AnyView, View};
use waterui_layout::padding::EdgeInsets;
use waterui_layout::stack::vstack;
use waterui_shape::ShapeExt as _;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};
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
        .into_snapshot()
        .expect("a pumped frame must be capturable")
}

/// The pixel at the centre of `rect`.
fn centre_pixel(snapshot: &HeadlessSnapshot, rect: Rect) -> [u8; 4] {
    let x = crate::num_cast::f64_as_u32(f64::midpoint(rect.x0, rect.x1));
    let y = crate::num_cast::f64_as_u32(f64::midpoint(rect.y0, rect.y1));
    let index = usize::try_from((y * snapshot.width() + x) * 4).expect("a pixel index fits usize");
    snapshot.rgba8()[index..index + 4]
        .try_into()
        .expect("four channels per pixel")
}

fn near(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.5
}

/// Writes `snapshot` under the test artifacts for visual review.
pub(super) fn save(snapshot: &HeadlessSnapshot, case: &str, name: &str) {
    let path = crate::testing::artifact_snapshot_path("hydrolysis", case, name);
    std::fs::create_dir_all(path.parent().expect("the case directory"))
        .expect("the capture directory must be creatable");
    image::RgbaImage::from_raw(
        snapshot.width(),
        snapshot.height(),
        snapshot.rgba8().to_vec(),
    )
    .expect("snapshot dimensions must match the rgba buffer")
    .save(&path)
    .expect("the frame must be writable");
}

/// A `when` branch that turns false: its instance leaves the accessibility
/// tree on the event frame, and the conditional's slot is sized by its live
/// content alone — now nothing — so the sibling below moves up at once. The
/// ghost fades at the frame it left at, beneath the sibling that now
/// overlaps it, until its clock completes.
#[test]
fn a_when_removal_leaves_semantics_at_once_and_its_ghost_overlaps_the_released_slot() {
    let show = Binding::container(true);
    let show_for_view = show.clone();
    let mut runtime = runtime(move || {
        AnyView::new(vstack((
            when(show_for_view.clone(), || {
                transitioned("leaving", RED, PropertyTransition::opacity())
            }),
            sized_swatch("sibling", BLUE, 120.0, 20.0),
        )))
    });
    let start = Instant::now();
    let rest = frame(&mut runtime, start);
    let leaving = bounds(&mut runtime, "leaving").expect("the branch renders at rest");
    // Inside the leaving swatch, below where the shorter sibling moves to.
    let uncovered = Rect::new(
        leaving.x0 + 50.0,
        leaving.y0 + 28.0,
        leaving.x0 + 70.0,
        leaving.y0 + 36.0,
    );

    show.set(false);
    let _ = frame(&mut runtime, start + Duration::from_millis(16));
    assert!(
        bounds(&mut runtime, "leaving").is_none(),
        "the removed instance leaves the accessibility tree on the event frame"
    );
    let moved = bounds(&mut runtime, "sibling").expect("the sibling renders mid-removal");
    assert!(
        near(moved.y0, leaving.y0),
        "the ghost takes no part in layout, so the sibling moves up into the \
         released slot on the event frame: leaving {leaving:?}, sibling {moved:?}"
    );
    let mid = frame(&mut runtime, start + Duration::from_millis(516));
    let gone = frame(&mut runtime, start + Duration::from_millis(1_100));
    let (from, to) = (
        centre_pixel(&rest, uncovered),
        centre_pixel(&gone, uncovered),
    );
    let ghost = centre_pixel(&mid, uncovered);
    assert!(
        between(ghost, from, to),
        "halfway through the removal the ghost fades at the frame it left at: rest \
         {from:?}, mid {ghost:?}, after {to:?}"
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
    // The window corner is background in every frame.
    let background = centre_pixel(&settled, Rect::new(0.0, 0.0, 2.0, 2.0));
    let (fading, solid) = (
        centre_pixel(&mid, entering),
        centre_pixel(&settled, entering),
    );
    assert!(
        between(fading, background, solid),
        "halfway through the insertion the instance fades in over its slot: \
         background {background:?}, mid {fading:?}, settled {solid:?}"
    );
}

/// A conditional directly in a stack is one stack member whatever it holds:
/// a transition declared over it swaps the incoming branch into the same
/// slot on the event frame, so a sibling whose position the live content
/// does not change stays put. The outgoing branch's ghost overlaps that slot
/// beneath the incoming branch, fading where it left.
#[test]
fn a_when_in_a_stack_swaps_in_place_with_its_ghost_beneath_the_new_branch() {
    let first = Binding::container(true);
    let first_for_view = first.clone();
    let mut runtime = runtime(move || {
        AnyView::new(vstack((
            when(first_for_view.clone(), || swatch("first", RED))
                .otherwise(|| sized_swatch("second", GREEN, 60.0, 40.0))
                .transition(PropertyTransition::opacity())
                .animation(Animation::linear(CLOCK)),
            swatch("sibling", BLUE),
        )))
    });
    let start = Instant::now();
    let rest = frame(&mut runtime, start);
    let slot = bounds(&mut runtime, "first").expect("the first branch renders at rest");
    let sibling = bounds(&mut runtime, "sibling").expect("the sibling renders at rest");
    // Inside the first swatch, beyond the narrower incoming branch.
    let uncovered = Rect::new(
        slot.x1 - 20.0,
        slot.y0 + 10.0,
        slot.x1 - 10.0,
        slot.y0 + 20.0,
    );

    first.set(false);
    let _ = frame(&mut runtime, start + Duration::from_millis(16));
    assert!(
        bounds(&mut runtime, "first").is_none(),
        "the swapped-out branch leaves the accessibility tree at once"
    );
    let second = bounds(&mut runtime, "second").expect("the new branch is in the tree");
    let stayed = bounds(&mut runtime, "sibling").expect("the sibling renders");
    assert!(
        near(second.y0, slot.y0) && near(stayed.y0, sibling.y0),
        "the new branch takes the conditional's one slot on the event frame and the \
         sibling stays put: slot {slot:?}, second {second:?}, sibling {sibling:?} -> \
         {stayed:?}"
    );
    let mid = frame(&mut runtime, start + Duration::from_millis(516));
    let gone = frame(&mut runtime, start + Duration::from_millis(1_100));
    let (from, to) = (
        centre_pixel(&rest, uncovered),
        centre_pixel(&gone, uncovered),
    );
    let ghost = centre_pixel(&mid, uncovered);
    assert!(
        between(ghost, from, to),
        "halfway through the swap the ghost fades over the slot: rest {from:?}, mid \
         {ghost:?}, after {to:?}"
    );
}

/// A declaration behind type erasure is still the root of the branch the
/// conditional swaps, so the conditional owns it: the erased instance leaves
/// as a ghost that keeps painting mid-flight, while the slot — sized by the
/// live content alone — is released on the event frame.
#[test]
fn a_declaration_behind_erasure_is_owned_by_the_conditional() {
    let show = Binding::container(true);
    let show_for_view = show.clone();
    let mut runtime = runtime(move || {
        AnyView::new(vstack((
            when(show_for_view.clone(), || {
                AnyView::new(AnyView::new(transitioned(
                    "leaving",
                    RED,
                    PropertyTransition::opacity(),
                )))
            }),
            sized_swatch("sibling", BLUE, 120.0, 20.0),
        )))
    });
    let start = Instant::now();
    let rest = frame(&mut runtime, start);
    let leaving = bounds(&mut runtime, "leaving").expect("the branch renders at rest");
    // Low inside the leaving swatch, below where the sibling moves to.
    let uncovered = Rect::new(
        f64::midpoint(leaving.x0, leaving.x1) - 2.0,
        leaving.y1 - 6.0,
        f64::midpoint(leaving.x0, leaving.x1) + 2.0,
        leaving.y1 - 2.0,
    );

    show.set(false);
    let _ = frame(&mut runtime, start + Duration::from_millis(16));
    assert!(bounds(&mut runtime, "leaving").is_none());
    let released = bounds(&mut runtime, "sibling").expect("the sibling renders mid-removal");
    assert!(
        near(released.y0, leaving.y0),
        "the slot is released on the event frame: leaving {leaving:?}, sibling {released:?}"
    );
    let mid = frame(&mut runtime, start + Duration::from_millis(516));
    let gone = frame(&mut runtime, start + Duration::from_millis(1_100));
    let (from, to) = (
        centre_pixel(&rest, uncovered),
        centre_pixel(&gone, uncovered),
    );
    let ghost = centre_pixel(&mid, uncovered);
    assert!(
        between(ghost, from, to),
        "the claimed declaration fades the erased ghost: rest {from:?}, mid {ghost:?}, \
         after {to:?}"
    );
}

/// A declaration beneath a modifier is not the root of the branch, so no
/// structural site claims it: the first frame fails and names the fix.
#[test]
#[should_panic(expected = "move `.transition(...)` outward so it is the outermost modifier")]
fn a_declaration_beneath_a_modifier_is_rejected() {
    let mut runtime = runtime(|| {
        AnyView::new(vstack((when(Binding::container(true), || {
            transitioned("padded", RED, PropertyTransition::opacity())
                .padding_with(EdgeInsets::all(8.0))
        }),)))
    });
    let _ = frame(&mut runtime, Instant::now());
}

/// A conditional at a collection row's root is that row's one member: when
/// it swaps, the row's slot is sized by the live content at once — the next
/// row moves on the event frame — and the outgoing branch's ghost fades at
/// the frame it left at, owned by the conditional rather than laid out by
/// the collection.
#[test]
fn a_conditional_at_a_collection_row_root_owns_its_ghost() {
    use nami::collection::List as Membership;
    use waterui::layout::stack::VStack;
    use waterui_core::id::SelfId;

    let first = Binding::container(true);
    let first_for_view = first.clone();
    let rows: Membership<SelfId<u64>> = Membership::from(vec![SelfId::new(1), SelfId::new(2)]);
    let mut runtime = runtime(move || {
        let first = first_for_view.clone();
        AnyView::new(VStack::for_each(rows.clone(), move |row| {
            if row.into_inner() == 1 {
                AnyView::new(
                    when(first.clone(), || swatch("leaving", RED))
                        .otherwise(|| sized_swatch("taller", GREEN, 60.0, 80.0))
                        .transition(PropertyTransition::opacity())
                        .animation(Animation::linear(CLOCK)),
                )
            } else {
                AnyView::new(swatch("next", BLUE))
            }
        }))
    });
    let start = Instant::now();
    let rest = frame(&mut runtime, start);
    let leaving = bounds(&mut runtime, "leaving").expect("the first row renders at rest");
    let next = bounds(&mut runtime, "next").expect("the second row renders at rest");
    // Inside the leaving swatch, beyond the narrower incoming branch.
    let uncovered = Rect::new(
        leaving.x1 - 20.0,
        leaving.y0 + 10.0,
        leaving.x1 - 10.0,
        leaving.y0 + 20.0,
    );

    first.set(false);
    let _ = frame(&mut runtime, start + Duration::from_millis(16));
    assert!(
        bounds(&mut runtime, "leaving").is_none(),
        "the ghost is absent from accessibility from the event frame"
    );
    let taller = bounds(&mut runtime, "taller").expect("the live branch renders");
    let moved = bounds(&mut runtime, "next").expect("the next row renders");
    assert!(
        near(taller.y0, leaving.y0) && near(moved.y0 - next.y0, 40.0),
        "the row's slot is sized by the live branch alone on the event frame: leaving \
         {leaving:?}, taller {taller:?}, next {next:?} -> {moved:?}"
    );
    let mid = frame(&mut runtime, start + Duration::from_millis(516));
    let gone = frame(&mut runtime, start + Duration::from_millis(1_100));
    let (from, to) = (
        centre_pixel(&rest, uncovered),
        centre_pixel(&gone, uncovered),
    );
    let ghost = centre_pixel(&mid, uncovered);
    assert!(
        between(ghost, from, to),
        "halfway through the swap the row's ghost fades at the frame it left at: rest \
         {from:?}, mid {ghost:?}, after {to:?}"
    );
}

/// A blur transition is realized as a property: halfway through the removal
/// the ghost draws through a Gaussian blur filter layer at the frame it left
/// at — its edge softens toward the background — beneath the sibling that
/// moved up into the released slot. The mid-flight frame lands under
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
            sized_swatch("sibling", BLUE, 120.0, 20.0),
        )))
    });
    let start = Instant::now();
    let rest = frame(&mut runtime, start);
    let leaving = bounds(&mut runtime, "leaving").expect("the branch renders at rest");

    show.set(false);
    let _ = frame(&mut runtime, start + Duration::from_millis(16));
    let moved = bounds(&mut runtime, "sibling").expect("the sibling renders mid-removal");
    assert!(
        near(moved.y0, leaving.y0),
        "the blurred ghost holds no slot: leaving {leaving:?}, sibling {moved:?}"
    );
    let mid = frame(&mut runtime, start + Duration::from_millis(516));
    save(&mid, "structural_transition_blur", "mid-removal");
    // A row below the sibling, one pixel inside the swatch's trailing edge:
    // solid at rest, blended with the background once the blur spreads the
    // edge.
    let row = leaving.y0 + 30.0;
    let edge = Rect::new(leaving.x1 - 2.0, row - 1.0, leaving.x1, row + 1.0);
    let centre = Rect::new(
        f64::midpoint(leaving.x0, leaving.x1) - 1.0,
        row - 1.0,
        f64::midpoint(leaving.x0, leaving.x1) + 1.0,
        row + 1.0,
    );
    assert!(
        centre_pixel(&mid, edge) != centre_pixel(&rest, edge),
        "the blurred ghost's edge softens mid-flight"
    );
    assert!(
        centre_pixel(&mid, centre) != centre_pixel(&mid, edge),
        "the ghost is blurred, not faded: its centre stays denser than its edge"
    );
}

/// A `width`x`height` swatch whose accessibility node carries `label`.
fn sized_swatch(label: &'static str, color: Srgb, width: f32, height: f32) -> impl View {
    RoundedRectangle::new(0.0)
        .fill(color)
        .a11y_label(label)
        .size(width, height)
}

/// Whether every channel of `mid` lies between `from` and `to`, with a
/// rounding allowance, and `mid` is neither end: a cross-fade, not a cut or
/// a slide.
fn between(mid: [u8; 4], from: [u8; 4], to: [u8; 4]) -> bool {
    let inside = mid
        .iter()
        .zip(from.iter().zip(to.iter()))
        .all(|(m, (f, t))| {
            let (low, high) = if f <= t { (f, t) } else { (t, f) };
            m.saturating_add(2) >= *low && *m <= high.saturating_add(2)
        });
    inside && mid != from && mid != to
}

/// Swaps a 120x40 branch for a narrower, taller one under the declaration
/// over the conditional, with `modifier` applied to the conditional, and
/// checks the rule that holds whatever layout encloses it: the ghost leaves
/// accessibility on the event frame, the slot is sized by the live content
/// alone (the sibling moves by the height difference at once), and the
/// ghost fades at the frame it left at. The mid-swap frame lands under
/// `<artifacts>/hydrolysis/<case>/mid-swap.png`.
fn assert_ghost_overlaps_under<V: View>(case: &str, modifier: impl Fn(AnyView) -> V + 'static) {
    let first = Binding::container(true);
    let first_for_view = first.clone();
    let mut runtime = runtime(move || {
        AnyView::new(vstack((
            modifier(AnyView::new(
                when(first_for_view.clone(), || swatch("leaving", RED))
                    .otherwise(|| sized_swatch("taller", GREEN, 60.0, 80.0))
                    .transition(PropertyTransition::opacity())
                    .animation(Animation::linear(CLOCK)),
            )),
            swatch("sibling", BLUE),
        )))
    });
    let start = Instant::now();
    let rest = frame(&mut runtime, start);
    let leaving = bounds(&mut runtime, "leaving").expect("the first branch renders at rest");
    let sibling = bounds(&mut runtime, "sibling").expect("the sibling renders at rest");
    // Inside the leaving swatch, beyond the narrower incoming branch.
    let uncovered = Rect::new(
        leaving.x1 - 20.0,
        leaving.y0 + 10.0,
        leaving.x1 - 10.0,
        leaving.y0 + 20.0,
    );

    first.set(false);
    let _ = frame(&mut runtime, start + Duration::from_millis(16));
    assert!(
        bounds(&mut runtime, "leaving").is_none(),
        "{case}: the ghost is absent from accessibility from the event frame"
    );
    let taller = bounds(&mut runtime, "taller").expect("the live branch renders");
    let moved = bounds(&mut runtime, "sibling").expect("the sibling renders");
    assert!(
        near(taller.y0, leaving.y0) && near(moved.y0 - sibling.y0, 40.0),
        "{case}: the slot is sized by the live content alone on the event frame: \
         leaving {leaving:?}, taller {taller:?}, sibling {sibling:?} -> {moved:?}"
    );
    let mid = frame(&mut runtime, start + Duration::from_millis(516));
    save(&mid, case, "mid-swap");
    let gone = frame(&mut runtime, start + Duration::from_millis(1_100));
    let (from, to) = (
        centre_pixel(&rest, uncovered),
        centre_pixel(&gone, uncovered),
    );
    let ghost = centre_pixel(&mid, uncovered);
    assert!(
        between(ghost, from, to),
        "{case}: halfway through the swap the ghost fades at the frame it left at: \
         rest {from:?}, mid {ghost:?}, after {to:?}"
    );
}

/// Under padding the conditional's ghost paints over the padded slot the
/// live content sizes.
#[test]
fn a_ghost_under_padding_overlaps_the_slot_the_live_content_sizes() {
    assert_ghost_overlaps_under("structural_transition_padding", |content| {
        content.padding_with(EdgeInsets::all(8.0))
    });
}

/// Under an alignment-guide override the conditional is still the
/// modifier's one child: the swap runs without the guide's layout ever
/// seeing the ghost.
#[test]
fn a_ghost_under_an_alignment_guide_overlaps_the_slot_the_live_content_sizes() {
    assert_ghost_overlaps_under("structural_transition_alignment_guide", |content| {
        content.horizontal_alignment_guide(HorizontalAlignment::Center, |dimensions| {
            dimensions.size.width / 2.0
        })
    });
}

/// Under a background the conditional is the content the background sizes
/// to: the background follows the live content and the ghost overlaps it.
#[test]
fn a_ghost_under_a_background_overlaps_the_slot_the_live_content_sizes() {
    assert_ghost_overlaps_under("structural_transition_background", |content| {
        content.background(Color::srgb(250, 250, 250))
    });
}

/// Drives `runtime` forward from `at` on a synthetic clock until it settles.
fn settle(runtime: &mut HeadlessRuntime, at: &mut Instant) {
    for _ in 0..400 {
        *at += Duration::from_millis(16);
        let _ = runtime.pump_at(true, *at);
        if runtime.is_settled() && !runtime.has_pending_semantic_update() {
            return;
        }
    }
    panic!("the runtime must settle within 400 frames");
}

/// Whether a navigation transition driven by a page root's own declaration
/// is still running at `now`.
fn page_transition_active(runtime: &HeadlessRuntime, now: Instant) -> bool {
    runtime.renderer().navigation.slots.values().any(|slot| {
        slot.transition
            .as_ref()
            .is_some_and(|transition| transition.page.is_some() && transition.is_active(now))
    })
}

/// A navigation page root's declared transition is its insertion on push
/// and its removal on pop, in place of the stack's slide: the stationary
/// root does not move, and the moving page cross-fades over it on its own
/// one-second clock. The arriving page is in the accessibility tree on the
/// push frame; the departing one leaves it on the pop frame.
#[test]
fn a_navigation_page_root_transition_replaces_the_stack_transition() {
    use waterui::navigation::{NavigationPath, NavigationStack, NavigationView};

    let path = NavigationPath::<i64>::new();
    let builder_path = path.clone();
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(
                NavigationStack::with_path(
                    builder_path.clone(),
                    NavigationView::new("Root", sized_swatch("root", BLUE, 300.0, 200.0)),
                )
                .destination(|_: i64| {
                    NavigationView::new(
                        "Page",
                        transitioned("page", RED, PropertyTransition::opacity()),
                    )
                }),
            )
        }),
        400,
        600,
        MinimalTestTheme::default(),
    );
    let mut at = Instant::now();
    settle(&mut runtime, &mut at);
    let root_rest = frame(&mut runtime, at);
    let root = bounds(&mut runtime, "root").expect("the root page renders");
    // Inside the root swatch, near its leading edge.
    let probe = Rect::new(root.x0 + 4.0, root.y0 + 4.0, root.x0 + 8.0, root.y0 + 8.0);

    path.push(1);
    let pushed_at = at + Duration::from_millis(16);
    let _ = frame(&mut runtime, pushed_at);
    assert!(
        bounds(&mut runtime, "page").is_some(),
        "the arriving page is in the accessibility tree on the push frame"
    );
    let mid_push = frame(&mut runtime, pushed_at + Duration::from_millis(500));
    assert!(
        page_transition_active(&runtime, pushed_at + Duration::from_millis(900)),
        "the push runs the page's own one-second clock, not the stack's"
    );
    let mut settled_at = pushed_at + Duration::from_millis(1_100);
    let page_rest = frame(&mut runtime, settled_at);
    settle(&mut runtime, &mut settled_at);
    let (from, to) = (
        centre_pixel(&root_rest, probe),
        centre_pixel(&page_rest, probe),
    );
    assert!(from != to, "the pushed page covers the probe once settled");
    let pixel = centre_pixel(&mid_push, probe);
    assert!(
        between(pixel, from, to),
        "mid-push the page fades in over a root that stays put: root {from:?}, mid \
         {pixel:?}, page {to:?}"
    );

    let _ = path.pop();
    let popped_at = settled_at + Duration::from_millis(16);
    let _ = frame(&mut runtime, popped_at);
    let mid_pop = frame(&mut runtime, popped_at + Duration::from_millis(500));
    assert!(
        bounds(&mut runtime, "page").is_none() && bounds(&mut runtime, "root").is_some(),
        "mid-pop the departing page is out of the accessibility tree and the root is in it"
    );
    assert!(
        page_transition_active(&runtime, popped_at + Duration::from_millis(900)),
        "the pop runs the page's own removal clock"
    );
    let pixel = centre_pixel(&mid_pop, probe);
    assert!(
        between(pixel, from, to),
        "mid-pop the page fades out over the stationary root: root {from:?}, mid \
         {pixel:?}, page {to:?}"
    );
    let mut done = popped_at + Duration::from_millis(1_100);
    let _ = frame(&mut runtime, done);
    settle(&mut runtime, &mut done);
    let back = frame(&mut runtime, done + Duration::from_millis(16));
    assert_eq!(
        centre_pixel(&back, probe),
        from,
        "after the pop the root is presented as it was"
    );
}

/// A `List` row owns the transition declared on its content: a removed row
/// leaves the accessibility tree at once, its ghost fades in the slot it
/// held while the next row stays put, and the slot is released when the
/// clock completes.
#[test]
fn a_list_row_removal_holds_its_slot_while_it_fades() {
    use nami::collection::List as Membership;
    use waterui::component::list::{List, ListItem};
    use waterui_core::id::SelfId;

    let mut rows: Membership<SelfId<u64>> =
        Membership::from(vec![SelfId::new(1), SelfId::new(2), SelfId::new(3)]);
    let rows_for_view = rows.clone();
    let mut runtime = runtime(move || {
        AnyView::new(List::for_each(rows_for_view.clone(), |row| {
            let (label, color) = match row.into_inner() {
                1 => ("row 1", RED),
                2 => ("row 2", GREEN),
                _ => ("row 3", BLUE),
            };
            ListItem::new(transitioned(label, color, PropertyTransition::opacity()))
        }))
    });
    let mut at = Instant::now();
    settle(&mut runtime, &mut at);
    let rest = frame(&mut runtime, at);
    let leaving = bounds(&mut runtime, "row 1").expect("the first row renders at rest");
    let next = bounds(&mut runtime, "row 2").expect("the second row renders at rest");

    let _ = rows.remove(0);
    let removed_at = at + Duration::from_millis(16);
    let _ = frame(&mut runtime, removed_at);
    assert!(
        bounds(&mut runtime, "row 1").is_none(),
        "the removed row leaves the accessibility tree on the event frame"
    );
    let mid = frame(&mut runtime, removed_at + Duration::from_millis(500));
    let held = bounds(&mut runtime, "row 2").expect("the next row renders mid-removal");
    assert!(
        near(held.y0, next.y0),
        "the ghost holds its row slot, so the next row stays put: rest {next:?}, mid {held:?}"
    );
    let gone = frame(&mut runtime, removed_at + Duration::from_millis(1_100));
    let mut after = removed_at + Duration::from_millis(1_116);
    settle(&mut runtime, &mut after);
    let released = bounds(&mut runtime, "row 2").expect("the next row renders after");
    assert!(
        near(released.y0, leaving.y0),
        "once the removal completes the next row moves up into the released slot: \
         leaving {leaving:?}, next {released:?}"
    );
    let ghost = centre_pixel(&mid, leaving);
    assert!(
        ghost != centre_pixel(&rest, leaving) && ghost != centre_pixel(&gone, leaving),
        "halfway through the removal the row's ghost is fading"
    );
}

/// A stack style that slides the moving page one viewport width toward the
/// trailing edge while the page beneath stays put: under an interactive pop
/// the departing page sits exactly `progress` widths to the right.
#[derive(Debug)]
struct EdgeSlide;

impl waterui::navigation::NavigationTransition for EdgeSlide {
    fn frame(
        &self,
        progress: f32,
        direction: waterui::navigation::NavigationTransitionDirection,
    ) -> waterui::navigation::NavigationTransitionFrame {
        use waterui::navigation::{
            NavigationTransitionDirection, NavigationTransitionFrame, NavigationTransitionLayer,
        };
        let moving = |offset_x| NavigationTransitionLayer::new(offset_x, 0.0, 1.0, 1.0);
        match direction {
            NavigationTransitionDirection::Push => NavigationTransitionFrame::new(
                NavigationTransitionLayer::IDENTITY,
                moving(1.0 - progress),
            ),
            NavigationTransitionDirection::Pop => NavigationTransitionFrame::new(
                moving(progress),
                NavigationTransitionLayer::IDENTITY,
            ),
        }
    }
}

const EDGE_POP_WIDTH: u16 = 400;
const EDGE_POP_POINTER: u64 = 7;
/// Where the edge drag presses: inside the stack's leading-edge strip.
const EDGE_POP_START: [f32; 2] = [10.0, 300.0];

/// A stack under the `EdgeSlide` style whose root shows a blue 300x200
/// swatch and whose pushed page shows `page`, with the root's settled frame
/// — the page has been pushed and has settled by the time this returns, at
/// `at`.
fn edge_pop_stack(
    page: impl Fn() -> AnyView + 'static,
    at: &mut Instant,
) -> (HeadlessRuntime, HeadlessSnapshot) {
    use waterui::navigation::{NavigationPath, NavigationStack, NavigationView};

    let path = NavigationPath::<i64>::new();
    let builder_path = path.clone();
    let page = std::rc::Rc::new(page);
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || {
            let page = std::rc::Rc::clone(&page);
            AnyView::new(
                NavigationStack::with_path(
                    builder_path.clone(),
                    NavigationView::new("Root", sized_swatch("root", BLUE, 300.0, 200.0)),
                )
                .transition(EdgeSlide)
                .destination(move |_: i64| NavigationView::new("Page", page())),
            )
        }),
        u32::from(EDGE_POP_WIDTH),
        600,
        MinimalTestTheme::default(),
    );
    settle(&mut runtime, at);
    let root_rest = frame(&mut runtime, *at);
    path.push(1);
    *at += Duration::from_millis(16);
    let _ = frame(&mut runtime, *at);
    settle(&mut runtime, at);
    (runtime, root_rest)
}

/// The pointer position `progress` of the stack's width past the press.
fn edge_point(progress: f32) -> [f32; 2] {
    let [x, y] = EDGE_POP_START;
    [progress.mul_add(f32::from(EDGE_POP_WIDTH), x), y]
}

/// Feeds one pointer event and renders its frame at `at`.
fn edge_pointer(runtime: &mut HeadlessRuntime, event: InputEvent, at: Instant) -> HeadlessSnapshot {
    runtime.push_input_event(event);
    frame(runtime, at)
}

fn edge_press(runtime: &mut HeadlessRuntime, at: Instant) -> HeadlessSnapshot {
    let [x, y] = EDGE_POP_START;
    edge_pointer(
        runtime,
        InputEvent::PointerDown {
            id: EDGE_POP_POINTER,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
        at,
    )
}

/// Drags the pressed edge to `progress` of the stack's width.
fn edge_drag_to(runtime: &mut HeadlessRuntime, progress: f32, at: Instant) -> HeadlessSnapshot {
    let [x, y] = edge_point(progress);
    edge_pointer(
        runtime,
        InputEvent::PointerMove {
            id: EDGE_POP_POINTER,
            kind: PointerKind::Mouse,
            x,
            y,
        },
        at,
    )
}

/// Releases the edge drag at `progress` of the stack's width.
fn edge_release(runtime: &mut HeadlessRuntime, progress: f32, at: Instant) -> HeadlessSnapshot {
    let [x, y] = edge_point(progress);
    edge_pointer(
        runtime,
        InputEvent::PointerUp {
            id: EDGE_POP_POINTER,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
        at,
    )
}

fn interactive_pop_in_flight(runtime: &HeadlessRuntime) -> bool {
    runtime
        .renderer()
        .navigation
        .slots
        .values()
        .any(|slot| slot.interactive_pop.is_some())
}

/// The pushed page of the edge-pop cases: a red 300x200 swatch over the
/// root's blue one, fading in and out under its root's own declaration.
fn faded_page() -> AnyView {
    AnyView::new(
        sized_swatch("page", RED, 300.0, 200.0)
            .transition(PropertyTransition::opacity())
            .animation(Animation::linear(CLOCK)),
    )
}

/// The probe both swatches cover: near the leading edge of the page's
/// swatch, left of where a 0.3-width slide would have moved the page to.
fn edge_probe(runtime: &mut HeadlessRuntime) -> Rect {
    let page = bounds(runtime, "page").expect("the pushed page renders");
    Rect::new(page.x0 + 4.0, page.y0 + 4.0, page.x0 + 8.0, page.y0 + 8.0)
}

/// An edge drag released below the commit threshold pops nothing: while it
/// drags, the departing page's root removal runs at the drag's progress —
/// a fade over the stationary root, not the stack's slide — and after the
/// release it reverses toward presence until the page is back as it was.
#[test]
fn an_edge_drag_released_early_reverses_the_page_root_removal() {
    let mut at = Instant::now();
    let (mut runtime, root_rest) = edge_pop_stack(faded_page, &mut at);
    let probe = edge_probe(&mut runtime);
    let page_rest = frame(&mut runtime, at);
    let page = centre_pixel(&page_rest, probe);
    let root = centre_pixel(&root_rest, probe);

    at += Duration::from_millis(16);
    let _ = edge_press(&mut runtime, at);
    at += Duration::from_millis(16);
    let dragged = centre_pixel(&edge_drag_to(&mut runtime, 0.3, at), probe);
    assert!(
        between(dragged, root, page),
        "dragged 0.3 of the way, the page fades by the drag's progress over a root that \
         stays put: root {root:?}, dragged {dragged:?}, page {page:?}"
    );
    assert!(
        bounds(&mut runtime, "page").is_some(),
        "a drag alone does not pop the page"
    );

    at += Duration::from_millis(16);
    let _ = edge_release(&mut runtime, 0.3, at);
    let returning = centre_pixel(&frame(&mut runtime, at + Duration::from_millis(50)), probe);
    assert!(
        between(returning, dragged, page),
        "after an early release the removal reverses from where the drag left it: \
         dragged {dragged:?}, returning {returning:?}, page {page:?}"
    );

    at += Duration::from_millis(50);
    settle(&mut runtime, &mut at);
    at += Duration::from_millis(16);
    let back = frame(&mut runtime, at);
    assert!(
        !interactive_pop_in_flight(&runtime),
        "the cancelled pop ended"
    );
    assert!(
        bounds(&mut runtime, "page").is_some(),
        "a cancelled edge drag keeps the page"
    );
    assert_eq!(
        centre_pixel(&back, probe),
        page,
        "the page is presented as it was before the drag"
    );
}

/// An edge drag released past the commit threshold pops the page: the
/// departing page's root removal runs at the drag's progress, then completes
/// from that progress after the release, and the root is presented as it was.
#[test]
fn an_edge_drag_released_late_completes_the_page_root_removal() {
    let mut at = Instant::now();
    let (mut runtime, root_rest) = edge_pop_stack(faded_page, &mut at);
    let probe = edge_probe(&mut runtime);
    let page_rest = frame(&mut runtime, at);
    let page = centre_pixel(&page_rest, probe);
    let root = centre_pixel(&root_rest, probe);

    at += Duration::from_millis(16);
    let _ = edge_press(&mut runtime, at);
    at += Duration::from_millis(16);
    let dragged = centre_pixel(&edge_drag_to(&mut runtime, 0.7, at), probe);
    assert!(
        between(dragged, root, page),
        "dragged 0.7 of the way, the page fades by the drag's progress over a root that \
         stays put: root {root:?}, dragged {dragged:?}, page {page:?}"
    );

    at += Duration::from_millis(16);
    let _ = edge_release(&mut runtime, 0.7, at);
    let leaving = centre_pixel(&frame(&mut runtime, at + Duration::from_millis(50)), probe);
    assert!(
        between(leaving, root, dragged),
        "after a late release the removal completes from where the drag left it: \
         root {root:?}, leaving {leaving:?}, dragged {dragged:?}"
    );

    at += Duration::from_millis(50);
    settle(&mut runtime, &mut at);
    at += Duration::from_millis(16);
    let back = frame(&mut runtime, at);
    assert!(
        !interactive_pop_in_flight(&runtime),
        "the committed pop ended"
    );
    assert!(
        bounds(&mut runtime, "page").is_none() && bounds(&mut runtime, "root").is_some(),
        "a committed edge drag pops the page"
    );
    assert_eq!(
        centre_pixel(&back, probe),
        root,
        "after the pop the root is presented as it was"
    );
}

/// Without a page-root declaration an edge drag still drives the stack's
/// style by the drag's progress: the departing page slides 0.3 of the width
/// toward the trailing edge, unfaded, and the root beneath is uncovered.
#[test]
fn an_edge_drag_without_a_page_root_declaration_drives_the_stack_style() {
    let mut at = Instant::now();
    let (mut runtime, root_rest) = edge_pop_stack(
        || AnyView::new(sized_swatch("page", RED, 300.0, 200.0)),
        &mut at,
    );
    let probe = edge_probe(&mut runtime);
    let page_rest = frame(&mut runtime, at);
    let page = centre_pixel(&page_rest, probe);
    let root = centre_pixel(&root_rest, probe);
    let travel = 0.3 * f64::from(EDGE_POP_WIDTH);
    let slid = Rect::new(probe.x0 + travel, probe.y0, probe.x1 + travel, probe.y1);

    at += Duration::from_millis(16);
    let _ = edge_press(&mut runtime, at);
    at += Duration::from_millis(16);
    let dragged = edge_drag_to(&mut runtime, 0.3, at);
    assert_eq!(
        centre_pixel(&dragged, slid),
        page,
        "the departing page slides by the drag's progress, unfaded"
    );
    assert_eq!(
        centre_pixel(&dragged, probe),
        root,
        "the slide uncovers the root beneath, which stays put"
    );

    at += Duration::from_millis(16);
    let _ = edge_release(&mut runtime, 0.3, at);
    settle(&mut runtime, &mut at);
    assert!(
        bounds(&mut runtime, "page").is_some(),
        "the early release keeps the page"
    );
}
