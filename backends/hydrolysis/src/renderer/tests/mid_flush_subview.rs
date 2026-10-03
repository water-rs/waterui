//! water-rs/hydrolysis#226 — a `Dynamic` (`when`) whose pending child applies
//! inside a retained sub-view whose bounds are imposed (navigation content is
//! laid out at its caller-fixed rect, never re-measured per frame) must never
//! encode the swapped subtree at the placement the *closed* branch earned.
//! Pre-fix the pending consumed itself mid-flush: `patch_built` next frame
//! never re-tripped `needs_layout`, so `.background(color)` painted nothing
//! while the collection's rows kept painting through the enclosing scroll's
//! lazy viewport.
//!
//! The fix removes the mid-flush apply for this repro shape entirely:
//! `LazyStackNode::patch_visible` materializes the ids the stored viewport
//! window covers — including ones a membership change slid into it — inside
//! the *patch* walk, so the setter row's builder lands `open`'s pending while
//! the walk still reaches the `when` host's patch arm (patch-phase swap →
//! layout → flush). Pending that genuinely lands mid-pass (a `set` from a
//! flush that runs after the host's patch arm) is still applied at the host's
//! own entry — but now lays the new child out inside the host's assigned rect
//! the same frame and marks the host layout-dirty, so the enclosing retained
//! sub-view re-lays out before its next flush instead of pinning the stale
//! placement forever.

use std::time::{Duration, Instant};

use nami::Binding;
use nami::Signal as _;
use nami::collection::SignalCollection;
use waterui::ViewExt as _;
use waterui::graphics::color::Srgb;
use waterui::prelude::text;
use waterui::shape::RoundedRectangle;
use waterui::theme::color::Surface;
use waterui::widget::condition::when;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::scroll::ScrollView;
use waterui_layout::stack::{VStack, vstack};
use waterui_navigation::NavigationView;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;

const WINDOW_WIDTH: u32 = 320;
const WINDOW_HEIGHT: u32 = 240;

fn runtime(builder: AnyViewBuilder<AnyView>) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

fn pump(runtime: &mut HeadlessRuntime, at: &mut Instant, frames: u32) {
    for _ in 0..frames {
        *at += Duration::from_millis(16);
        runtime.pump_at(false, *at);
    }
}

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

/// The painted `y` extent of one exact colour: `(first row, last row, pixels)`.
fn ink_span(rgba8: &[u8], rgb: [u8; 3]) -> Option<(usize, usize, usize)> {
    let mut y0 = usize::MAX;
    let mut y1 = 0;
    let mut n = 0;
    for (y, row) in rgba8
        .as_chunks::<{ WINDOW_WIDTH as usize * 4 }>()
        .0
        .iter()
        .enumerate()
    {
        for px in row.as_chunks::<4>().0 {
            if px[..3] == rgb[..] {
                y0 = y0.min(y);
                y1 = y1.max(y);
                n += 1;
            }
        }
    }
    (n > 0).then_some((y0, y1, n))
}

/// `Surface` resolved under the test environment's installed default tokens.
const SURFACE: [u8; 3] = [0xFF, 0xFF, 0xFF];
const SETTER_ROW: [u8; 3] = [0x40, 0x40, 0x40];
const INNER_ROW_A: [u8; 3] = [0x40, 0x60, 0x40];
const INNER_ROW_B: [u8; 3] = [0x60, 0x40, 0x60];

/// The popup subtree inside navigation content: `open` flips from a lazy row
/// builder — a lazy inner row appended to a content-sized stack materializes
/// and its builder sets `open`. With `patch_visible` materializing the row
/// inside the patch walk, the pending reaches the `when` host's patch arm and
/// the swap applies before the frame's layout — so the *first* frame after
/// the toggle must already paint the background at the new bounds. (The
/// residual mid-pass path — pending landing after the host's patch arm — is
/// covered by the host re-laying out its child inside the assigned rect and
/// invalidating the enclosing sub-view.)
///
/// The rows carry plain `.background` colours so the painted bands are
/// pixel-visible headless; the assertions below check the first post-toggle
/// frame for row overlap — a regressed layout stacked the inner stack's
/// honest measure inside a slot it could not fit, pushing a row up over the
/// setter row — and that each row keeps its measured height.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
fn mid_flush_dynamic_apply_in_retained_subview_relayouts() {
    let open = Binding::bool(false);
    let inner_items: Binding<Vec<SelfId<u64>>> = Binding::container(vec![SelfId::new(7_u64)]);
    let messages = SignalCollection::new(Binding::container(
        (0..3_u64).map(SelfId::new).collect::<Vec<_>>(),
    ));

    let builder = {
        let open = open;
        let inner_items = inner_items.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let open = open.clone();
            let inner_items = inner_items.clone();
            AnyView::new(
                NavigationView::new(
                    "Chat",
                    ScrollView::vertical(vstack((
                        VStack::for_each(messages.clone(), {
                            let open = open.clone();
                            move |m: SelfId<u64>| {
                                let open = open.clone();
                                let inner_items = inner_items.clone();
                                if m.into_inner() == 2 {
                                    // Setter row: a lazy stack that gains a row
                                    // when `inner_items` grows; the row's builder
                                    // runs at materialization and sets `open`.
                                    // The rows size to content like the real
                                    // popup — a fixed-height clamp here would
                                    // under-provision them and `Frame` would
                                    // center the overflow into the sibling row.
                                    AnyView::new(vstack((
                                        text("setter row")
                                            .body()
                                            .background(Srgb::from_hex("#404040")),
                                        VStack::for_each(
                                            SignalCollection::new(inner_items.clone()),
                                            move |i: SelfId<u64>| {
                                                if inner_items.snapshot().len() > 1 {
                                                    open.set(true);
                                                }
                                                text("inner").body().background(Srgb::from_hex(
                                                    if i.into_inner() == 7 {
                                                        "#406040"
                                                    } else {
                                                        "#604060"
                                                    },
                                                ))
                                            },
                                        ),
                                    )))
                                } else {
                                    AnyView::new(text(format!("message {}", m.into_inner())).body())
                                }
                            }
                        }),
                        when(open, || {
                            VStack::for_each(
                                SignalCollection::new(Binding::container(
                                    (1..=2_u64).map(SelfId::new).collect::<Vec<_>>(),
                                )),
                                move |id: SelfId<u64>| {
                                    text(format!("suggestion {}", id.into_inner())).body()
                                },
                            )
                            .spacing(0.0)
                            .padding_with((4.0, 0.0))
                            .background(Surface)
                            .clip(RoundedRectangle::new(6.0))
                        }),
                        text("tail").body(),
                    ))),
                )
                .background(Srgb::from_hex("#141218")),
            )
        })
    };

    let mut runtime = runtime(builder);
    let mut at = Instant::now();
    pump(&mut runtime, &mut at, 6);

    // How much `Surface` the closed tree paints — the navigation chrome may
    // legitimately use the token; the pill's fill must exceed that baseline.
    at += Duration::from_millis(16);
    let baseline = count_rgb(
        &runtime
            .pump_at(true, at)
            .snapshot
            .expect("the frame must capture")
            .rgba8,
        SURFACE,
        2,
    );

    // Append the setter row: its builder fires `open.set` while materializing.
    let mut items = inner_items.snapshot();
    items.push(SelfId::new(9_u64));
    inner_items.set(items);

    // The FIRST frame after the toggle already paints the background at the
    // new bounds — a swap may not encode a stale placement even for one frame.
    at += Duration::from_millis(16);
    let snapshot = runtime
        .pump_at(true, at)
        .snapshot
        .expect("the frame must capture");
    let painted = count_rgb(&snapshot.rgba8, SURFACE, 2);
    assert!(
        painted > baseline + 200,
        "the first frame after `open` applies must already paint the `when` \
         subtree's `.background(Surface)` at its new bounds; got {painted} \
         Surface px over a {baseline} px baseline — the swap encoded at the \
         closed branch's stale placement"
    );
    // The paint must sit below the message rows — where the pill's new bounds
    // are — not at some stale placement elsewhere in the sub-view.
    let lower_painted = count_rgb(
        &snapshot.rgba8[(WINDOW_WIDTH as usize) * 4 * 70..],
        SURFACE,
        2,
    );
    assert!(
        lower_painted > 200,
        "the `when` subtree must paint inside the content area below the \
         message rows; got {lower_painted} Surface px below y=70"
    );

    // The first post-toggle frame must not stack rows on top of each other:
    // every painted band keeps its measured height (~18.75pt here) and the
    // bands stay disjoint — the appended inner row belongs *below* the first.
    let setter = ink_span(&snapshot.rgba8, SETTER_ROW).expect("setter row painted");
    let inner_a = ink_span(&snapshot.rgba8, INNER_ROW_A).expect("inner row A painted");
    let inner_b = ink_span(&snapshot.rgba8, INNER_ROW_B).expect("inner row B painted");
    assert!(
        inner_a.0 > setter.1,
        "inner row A (y{}..{}) must start below the setter row (y{}..{}); \
         a row was placed at a stale height on the toggle frame",
        inner_a.0,
        inner_a.1,
        setter.0,
        setter.1
    );
    assert!(
        inner_b.0 > inner_a.1,
        "inner row B (y{}..{}) must start below inner row A (y{}..{}); \
         the rows' painted frames overlap",
        inner_b.0,
        inner_b.1,
        inner_a.0,
        inner_a.1
    );
    for (name, (y0, y1, _)) in [
        ("setter", setter),
        ("inner A", inner_a),
        ("inner B", inner_b),
    ] {
        let height = y1 + 1 - y0;
        assert!(
            (12..=22).contains(&height),
            "{name} row painted {height}px tall; expected its measured ~18.75pt — \
             the toggle frame placed it at a stale or collapsed height"
        );
    }

    pump(&mut runtime, &mut at, 12);

    let snapshot = runtime
        .pump_at(true, at)
        .snapshot
        .expect("the frame must capture");
    let painted = count_rgb(&snapshot.rgba8, SURFACE, 2);
    assert!(
        painted > baseline + 200,
        "the `when` subtree's `.background(Surface)` must keep painting after \
         `open` applies; got {painted} Surface px over a {baseline} px baseline"
    );

    // Steady state keeps the same row placement invariants.
    let setter = ink_span(&snapshot.rgba8, SETTER_ROW).expect("setter row painted");
    let inner_a = ink_span(&snapshot.rgba8, INNER_ROW_A).expect("inner row A painted");
    let inner_b = ink_span(&snapshot.rgba8, INNER_ROW_B).expect("inner row B painted");
    assert!(
        inner_a.0 > setter.1 && inner_b.0 > inner_a.1,
        "painted rows must stay disjoint: setter y{}..{}, A y{}..{}, B y{}..{}",
        setter.0,
        setter.1,
        inner_a.0,
        inner_a.1,
        inner_b.0,
        inner_b.1
    );
}
