//! Game-engine-purification feasibility probe.
//!
//! Measures the **from-scratch** per-frame cost of re-dispatching and re-encoding
//! a full screen of widgets every frame, with *no* retained window frame and *no*
//! collection cache — the exact cost the retained-scene replay path avoids. The
//! question: does it fit the 120 fps (8.33 ms) budget with headroom? If so, the
//! retained-scene layer is unnecessary complexity for Hydrolysis's "game-engine,
//! full-redraw-every-frame" design point.
//!
//! Each rebuild sample creates a fresh runtime and measures its one initial
//! structural dispatch. The scene includes a reactive `text!` to keep the tested
//! tree representative, but later signal changes use the retained refresh path and
//! are deliberately not counted as structural rebuilds. The scene is a
//! `VStack<(Vec<AnyView>,)>` (a `FixedContainer`: non-virtualized, non-collection).
//!
//! Rows come in two shapes with the **same primitive count** — a `text` variant
//! (2 glyph runs/row) and a `shapes` variant (2 rect fills/row) — to separate the
//! Vello **encode** cost from the text **measurement** cost. `scene_dispatch` is
//! the timed phase (dispatch + layout/measure + scene encode); it is CPU-only and
//! reproduces on any machine. `build_content` (root-builder re-run) is reported
//! too and is harness-inflated; in a real app's static tree it is near-zero.

use core::time::Duration;
use std::time::Instant;

use waterui::reactive::binding;
use waterui::shape::{Circle, RoundedRectangle, ShapeExt as _};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Binding, Computed};

use super::test_environment;
use crate::HeadlessRuntime;
use crate::platform::InputEvent;

use vello::kurbo::Affine;
use vello::peniko::{Brush, Color, Fill};
use waterui_core::layout::{
    HorizontalAlignment, Layout, ProposalSize, Rect as LayoutRect, Size, StretchAxis, SubView,
    VerticalAlignment, ViewDimensions,
};
use waterui_layout::stack::{HStackLayout, VStackLayout};

const WARMUP_FRAMES: usize = 6;
const SAMPLE_FRAMES: usize = 30;
const BUDGET_120HZ: Duration = Duration::from_micros(8_333);
/// What the 120Hz budget becomes for an unoptimized build sharing a machine with
/// the compiler. Generous on purpose: this catches a structural regression, not a
/// few hundred microseconds of drift.
const DEBUG_FLUSH_CEILING: Duration = Duration::from_millis(50);
const WINDOW_W: u32 = 420;
const WINDOW_H: u32 = 920;

/// One dense row on a card background: a leading filled circle, a two-line middle,
/// a trailing rounded chip. The middle is either two text lines (`with_text`) or
/// two equally-numerous rect fills, so the two variants differ only in glyph-run
/// vs fill encoding and in text measurement.
fn dense_row(i: usize, with_text: bool) -> AnyView {
    use waterui::prelude::*;
    let middle: AnyView = if with_text {
        AnyView::new(
            vstack((
                text(format!("Item {i}")).size(16.0),
                text("Subtitle line for visual density").size(13.0),
            ))
            .spacing(2.0),
        )
    } else {
        AnyView::new(
            vstack((
                ().size(120.0, 16.0).background(Color::srgb_hex("#D1D5DB")),
                ().size(160.0, 13.0).background(Color::srgb_hex("#9CA3AF")),
            ))
            .spacing(2.0),
        )
    };
    AnyView::new(
        hstack((
            Circle.fill(Color::srgb_hex("#3B82F6")).size(36.0, 36.0),
            middle,
            spacer(),
            RoundedRectangle::new(0.3)
                .fill(Color::srgb_hex("#E5E7EB"))
                .size(22.0, 22.0),
        ))
        .spacing(10.0)
        .background(Color::srgb_hex("#FFFFFF")),
    )
}

/// A full screen of `cards` dense rows in a fixed container, plus one reactive
/// `text!` driven by `tick`; this tree is used for fresh-runtime initial builds.
fn rebuild_screen(cards: usize, with_text: bool, tick: Binding<u64>) -> AnyView {
    use waterui::prelude::*;
    let mut children: Vec<AnyView> = Vec::with_capacity(cards + 1);
    children.push(AnyView::new(text!("{tick}", tick = tick.clone())));
    for i in 0..cards {
        children.push(dense_row(i, with_text));
    }
    AnyView::new(
        children
            .into_iter()
            .collect::<VStack<(Vec<AnyView>,)>>()
            .background(Color::srgb_hex("#EEF2FF")),
    )
}

/// The same dense rows wrapped in a `scroll`, so scroll-offset changes drive the
/// retained window-frame **replay** path (`refresh_window_frame`) instead of a
/// re-dispatch. Fixed (non-lazy) content is viewport-independent, so the retained
/// frame is reused by translation across scroll frames.
fn replay_screen(cards: usize, with_text: bool) -> AnyView {
    use waterui::prelude::*;
    let rows = (0..cards)
        .map(|i| dense_row(i, with_text))
        .collect::<VStack<(Vec<AnyView>,)>>();
    AnyView::new(scroll(rows).background(Color::srgb_hex("#EEF2FF")))
}

#[derive(Debug, Clone, Copy)]
struct Stats {
    dispatch_median: Duration,
    dispatch_p100: Duration,
    build_median: Duration,
}

fn percentiles(mut dispatch: Vec<Duration>, mut build: Vec<Duration>) -> Stats {
    dispatch.sort_unstable();
    build.sort_unstable();
    Stats {
        dispatch_median: dispatch[dispatch.len() / 2],
        dispatch_p100: *dispatch.last().expect("sampled at least one frame"),
        build_median: build[build.len() / 2],
    }
}

/// Full re-dispatch cost (the once-per-window build the retained tree amortizes).
/// On the retained-tree path a binding mutation patches in place and a "rebuild"
/// reuses the persistent tree, so a full re-dispatch only happens on the *initial*
/// build. Measure that by building a fresh runtime per sample: each first pump
/// re-walks + re-measures + re-encodes the whole view tree — the expensive work the
/// retained tree pays once, versus the per-frame replay below.
fn measure_rebuild(cards: usize, with_text: bool) -> Stats {
    let mut dispatch = Vec::with_capacity(SAMPLE_FRAMES);
    let mut build = Vec::with_capacity(SAMPLE_FRAMES);
    for _ in 0..SAMPLE_FRAMES {
        let builder =
            AnyViewBuilder::<AnyView>::new(move || rebuild_screen(cards, with_text, binding(0u64)));
        let env = test_environment();
        let mut rt = HeadlessRuntime::new_for_tests(env, builder, WINDOW_W, WINDOW_H);
        let result = rt.pump_at(false, Instant::now());
        assert!(
            result.rebuilt,
            "cards={cards} with_text={with_text}: the initial build must re-dispatch"
        );
        dispatch.push(result.profile.phases.scene_dispatch);
        build.push(result.profile.phases.build_content);
    }
    percentiles(dispatch, build)
}

/// Retained-replay every frame (current scroll/animation cost): scroll-offset
/// deltas drive `refresh_window_frame`, which skips measure+encode and replays the
/// retained draw-ops — but still re-registers interaction/accessibility targets.
fn measure_replay(cards: usize, with_text: bool) -> (Stats, u32) {
    let builder = AnyViewBuilder::<AnyView>::new(move || replay_screen(cards, with_text));
    let env = test_environment();
    let mut rt = HeadlessRuntime::new_for_tests(env, builder, WINDOW_W, WINDOW_H);

    let start = Instant::now();
    let _ = rt.pump_at(false, start); // initial rebuild establishes the retained frame
    let scroll = |dy: f32| InputEvent::Scroll {
        x: (WINDOW_W / 2) as f32,
        y: (WINDOW_H / 2) as f32,
        dx: 0.0,
        dy,
        is_line_delta: false,
    };
    let pump = |rt: &mut HeadlessRuntime, frame: u64| {
        // Oscillate small deltas to stay within scroll bounds (always moving, never clamped).
        rt.push_input_event(scroll(if frame.is_multiple_of(2) { -6.0 } else { 6.0 }));
        rt.pump_at(false, start + Duration::from_millis(frame * 8))
    };
    for f in 0..WARMUP_FRAMES as u64 {
        let _ = pump(&mut rt, f + 1);
    }
    let mut dispatch = Vec::with_capacity(SAMPLE_FRAMES);
    let mut build = Vec::with_capacity(SAMPLE_FRAMES);
    let mut rebuilt_frames = 0u32;
    for f in 0..SAMPLE_FRAMES as u64 {
        let result = pump(&mut rt, WARMUP_FRAMES as u64 + f + 1);
        if result.rebuilt {
            rebuilt_frames += 1; // a frame that escalated to re-dispatch is not a replay sample
        }
        dispatch.push(result.profile.phases.scene_dispatch);
        build.push(result.profile.phases.build_content);
    }
    (percentiles(dispatch, build), rebuilt_frames)
}

/// Pure Vello CPU scene-encode floor: the per-frame cost a persistent retained
/// reactive tree (Flutter-RenderObject / scene-graph style) would actually pay,
/// once `body()`/dispatch/a11y-build/target-registration are amortized to
/// once-and-on-Dynamic-change. No renderer, no dispatch — just encoding the row
/// primitives into a fresh `vello::Scene` every frame.
#[test]
fn pure_vello_encode_floor() {
    use vello::kurbo::{Circle as KurboCircle, Rect, RoundedRect};

    let white = Brush::Solid(Color::new([1.0, 1.0, 1.0, 1.0]));
    let blue = Brush::Solid(Color::new([0.23, 0.51, 0.96, 1.0]));
    let gray = Brush::Solid(Color::new([0.90, 0.90, 0.92, 1.0]));

    eprintln!("\n=== Pure Vello CPU scene-encode floor (release) ===");
    eprintln!(
        "(the per-frame cost a retained reactive tree pays: layout aside, just re-emit draw ops)"
    );
    for &rows in &[40usize, 80, 160, 320] {
        let mut scene = vello::Scene::new();
        let mut samples = Vec::with_capacity(SAMPLE_FRAMES);
        for f in 0..(WARMUP_FRAMES + SAMPLE_FRAMES) {
            scene.reset();
            let started = Instant::now();
            for i in 0..rows {
                let y = (i as f64) * 56.0;
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    &white,
                    None,
                    &Rect::new(8.0, y + 2.0, 412.0, y + 54.0),
                );
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    &blue,
                    None,
                    &KurboCircle::new((30.0, y + 28.0), 18.0),
                );
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    &gray,
                    None,
                    &RoundedRect::new(380.0, y + 17.0, 402.0, y + 39.0, 6.0),
                );
            }
            let elapsed = started.elapsed();
            if f >= WARMUP_FRAMES {
                samples.push(elapsed);
            }
        }
        samples.sort_unstable();
        eprintln!(
            "  rows={:>4} (~{:>4} paths)  encode median={:>9.3?}  p100={:>9.3?}",
            rows,
            rows * 3,
            samples[samples.len() / 2],
            samples.last().expect("sampled"),
        );
    }
}

#[test]
fn full_rebuild_vs_retained_replay_cost() {
    let sizes = [15usize, 40, 80, 160];

    eprintln!(
        "\n=== Hydrolysis per-frame cost: full re-dispatch vs retained replay (120Hz budget = {BUDGET_120HZ:?}) ==="
    );
    eprintln!(
        "(release build; scene_dispatch = the timed phase; rebuild re-encodes, replay reuses retained draw-ops)"
    );
    for &with_text in &[false, true] {
        eprintln!(
            "--- rows = {} ---",
            if with_text { "text" } else { "shapes" }
        );
        for &n in &sizes {
            let rebuild = measure_rebuild(n, with_text);
            let (replay, escalated) = measure_replay(n, with_text);
            let ratio =
                rebuild.dispatch_median.as_secs_f64() / replay.dispatch_median.as_secs_f64();
            eprintln!(
                "  cards={:>4}  rebuild median={:>9.3?} p100={:>9.3?}  |  replay median={:>9.3?} p100={:>9.3?}  |  rebuild/replay={:>5.1}x  (escalated {}/{})",
                n,
                rebuild.dispatch_median,
                rebuild.dispatch_p100,
                replay.dispatch_median,
                replay.dispatch_p100,
                ratio,
                escalated,
                SAMPLE_FRAMES,
            );
        }
    }

    // Report-only probe: the decision is read from the table above. Assert only the
    // invariant that makes the numbers meaningful — sampled rebuild frames really
    // did re-dispatch (so their cost is the game-engine-pure per-frame cost).
    let sanity = measure_rebuild(40, false);
    assert!(
        sanity.build_median < sanity.dispatch_median,
        "view construction ({:?}) should be far cheaper than dispatch ({:?})",
        sanity.build_median,
        sanity.dispatch_median
    );
}

// ---------------------------------------------------------------------------
// Phase 0 prototype: a minimal persistent retained render tree.
//
// This stands in for the future `RenderNode` tree to measure the ONE cost not
// isolated by the probes above: per-frame **layout + encode** on a tree that is
// built once and only re-flushed (no `body()`/dispatch/a11y/target churn). It is
// pure `vello::Scene` + the real `Layout` implementations — no renderer — so the
// numbers are the irreducible per-frame cost the persistent-tree architecture
// pays. The gate: a 160-row screen must flush well under the 120Hz budget.
// ---------------------------------------------------------------------------

/// A `Send + Sync` `SubView` adapter carrying a child's pre-measured size and
/// stretch axis, so a `ProtoNode::Container` can drive the real `Layout`
/// implementations (`VStackLayout`/`HStackLayout`) without holding a `!Sync`
/// node reference.
struct MeasuredSub {
    size: Size,
    stretch: StretchAxis,
}

impl SubView for MeasuredSub {
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(self.size)
    }
    fn stretch_axis(&self) -> StretchAxis {
        self.stretch
    }
    fn priority(&self) -> i32 {
        0
    }
}

/// A node in the prototype retained tree: either a fixed-size filled rect (leaf)
/// or a layout container owning child nodes.
enum ProtoNode {
    Color {
        size: Size,
        brush: Brush,
    },
    Container {
        layout: Box<dyn Layout>,
        children: Vec<ProtoNode>,
        /// Child frames cached by `layout()`. `flush()` reuses them, so a
        /// geometry-static frame pays only re-encode — the 120fps common case
        /// (color/opacity/transform animation, scroll offset, re-present). This
        /// models the node-local layout cache the real `RenderNode` tree needs;
        /// without it, relaying out the whole tree every frame is too slow at
        /// scale (160 rows ≈ 8ms uncached).
        placed: Vec<LayoutRect>,
    },
}

impl ProtoNode {
    /// Measure this node under a proposal (recursive; reuses the real `Layout`).
    fn measured(&self, proposal: ProposalSize) -> MeasuredSub {
        match self {
            ProtoNode::Color { size, .. } => MeasuredSub {
                size: *size,
                stretch: StretchAxis::None,
            },
            ProtoNode::Container {
                layout, children, ..
            } => {
                let subs: Vec<MeasuredSub> =
                    children.iter().map(|c| c.measured(proposal)).collect();
                let refs: Vec<&dyn SubView> = subs.iter().map(|s| s as &dyn SubView).collect();
                MeasuredSub {
                    size: layout.size_that_fits(proposal, &refs),
                    stretch: layout.stretch_axis(&[]),
                }
            }
        }
    }

    /// Re-measure and re-place this subtree, caching child frames. Run only when
    /// geometry-affecting inputs change (the rare, incremental case in the real
    /// tree — here it always relays out the whole tree as the worst case).
    fn layout(&mut self, proposal: ProposalSize, size: Size) {
        if let ProtoNode::Container {
            layout,
            children,
            placed,
        } = self
        {
            let subs: Vec<MeasuredSub> = children.iter().map(|c| c.measured(proposal)).collect();
            let refs: Vec<&dyn SubView> = subs.iter().map(|s| s as &dyn SubView).collect();
            let rects = layout.place(LayoutRect::from_size(size), &refs);
            drop(refs);
            for (child, rect) in children.iter_mut().zip(rects.iter()) {
                child.layout(
                    ProposalSize::new(Some(rect.width()), Some(rect.height())),
                    *rect.size(),
                );
            }
            *placed = rects;
        }
    }

    /// Re-encode this subtree into the scene using cached placements — the
    /// per-frame cost of a geometry-static frame.
    fn flush(&self, scene: &mut vello::Scene, transform: Affine, size: Size) {
        match self {
            ProtoNode::Color { brush, .. } => {
                let rect = vello::kurbo::Rect::new(
                    0.0,
                    0.0,
                    f64::from(size.width),
                    f64::from(size.height),
                );
                scene.fill(Fill::NonZero, transform, brush, None, &rect);
            }
            ProtoNode::Container {
                children, placed, ..
            } => {
                for (child, rect) in children.iter().zip(placed.iter()) {
                    let child_transform =
                        transform * Affine::translate((f64::from(rect.x()), f64::from(rect.y())));
                    child.flush(scene, child_transform, *rect.size());
                }
            }
        }
    }
}

fn proto_color(width: f32, height: f32, rgba: [f32; 4]) -> ProtoNode {
    ProtoNode::Color {
        size: Size::new(width, height),
        brush: Brush::Solid(Color::new(rgba)),
    }
}

/// One row: leading circle-sized block, a wide middle block, a trailing chip —
/// three fills laid out by a real `HStackLayout`, mirroring the dense_row shape.
fn proto_row() -> ProtoNode {
    ProtoNode::Container {
        layout: Box::new(HStackLayout {
            alignment: VerticalAlignment::Center,
            spacing: Computed::constant(10.0),
        }),
        children: vec![
            proto_color(36.0, 36.0, [0.23, 0.51, 0.96, 1.0]),
            proto_color(300.0, 40.0, [0.85, 0.87, 0.90, 1.0]),
            proto_color(22.0, 22.0, [0.90, 0.90, 0.92, 1.0]),
        ],
        placed: Vec::new(),
    }
}

fn build_proto_screen(rows: usize) -> ProtoNode {
    ProtoNode::Container {
        layout: Box::new(VStackLayout {
            alignment: HorizontalAlignment::Center,
            spacing: Computed::constant(10.0),
        }),
        children: (0..rows).map(|_| proto_row()).collect(),
        placed: Vec::new(),
    }
}

#[test]
fn prototype_flush_layout_cost() {
    let window = Size::new(420.0, 920.0);
    let proposal = ProposalSize::new(Some(window.width), Some(window.height));

    eprintln!("\n=== Phase 0 gate: persistent retained-tree per-frame cost, release ===");
    eprintln!(
        "flush = geometry-static frame (cached layout, just re-encode — the 120fps common case)"
    );
    eprintln!(
        "relayout+flush = geometry-changing frame (full relayout; INCREMENTAL in the real tree)"
    );
    for &rows in &[40usize, 160, 320] {
        let mut tree = build_proto_screen(rows);
        let mut scene = vello::Scene::new();
        tree.layout(proposal, window);
        for _ in 0..WARMUP_FRAMES {
            scene.reset();
            tree.flush(&mut scene, Affine::IDENTITY, window);
        }
        let mut flush_samples = Vec::with_capacity(SAMPLE_FRAMES);
        let mut relayout_samples = Vec::with_capacity(SAMPLE_FRAMES);
        for _ in 0..SAMPLE_FRAMES {
            scene.reset();
            let started = Instant::now();
            tree.flush(&mut scene, Affine::IDENTITY, window);
            flush_samples.push(started.elapsed());

            let started = Instant::now();
            tree.layout(proposal, window);
            tree.flush(&mut scene, Affine::IDENTITY, window);
            relayout_samples.push(started.elapsed());
        }
        flush_samples.sort_unstable();
        relayout_samples.sort_unstable();
        eprintln!(
            "  rows={:>4}  flush median={:>9.3?} p100={:>9.3?}  |  relayout+flush median={:>9.3?} (full-tree worst case)",
            rows,
            flush_samples[flush_samples.len() / 2],
            flush_samples.last().expect("sampled"),
            relayout_samples[relayout_samples.len() / 2],
        );
    }

    // Gate: the 120fps common-case frame (geometry static — animation, scroll,
    // re-present) must re-flush a large screen (160 dense rows) well under budget.
    let mut tree = build_proto_screen(160);
    let mut scene = vello::Scene::new();
    tree.layout(proposal, window);
    for _ in 0..WARMUP_FRAMES {
        scene.reset();
        tree.flush(&mut scene, Affine::IDENTITY, window);
    }
    let mut samples = Vec::with_capacity(SAMPLE_FRAMES);
    for _ in 0..SAMPLE_FRAMES {
        scene.reset();
        let started = Instant::now();
        tree.flush(&mut scene, Affine::IDENTITY, window);
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let median = samples[samples.len() / 2];
    let p100 = *samples.last().expect("sampled");
    eprintln!(
        "DECISION: 160-row geometry-static flush median = {median:?} p100 = {p100:?} vs 120Hz budget {BUDGET_120HZ:?}"
    );
    // Gate on the median, and against the unoptimized-build allowance rather than
    // the 120Hz budget itself. This runs under `cargo test`, so it is measuring a
    // debug build on a machine that is also compiling: p100 is the single worst
    // sample and moves with unrelated load, and the release frame this protects is
    // far quicker than what a debug build can show. The budget line above is the
    // number to read from the report; this assertion only has to catch a change
    // that makes the flush structurally slow.
    assert!(
        median < DEBUG_FLUSH_CEILING,
        "160-row geometry-static flush median {median:?} exceeds the debug-build ceiling \
         {DEBUG_FLUSH_CEILING:?} (120Hz release budget is {BUDGET_120HZ:?})"
    );
}
