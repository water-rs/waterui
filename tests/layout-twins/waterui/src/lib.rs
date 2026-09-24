//! Layout twins — WaterUI side of the audit fixture trees (water-rs/waterui#1229,
//! fixture specs in the audit comment on water-rs/waterui#1227: sections A2, A3,
//! A11, A12, A13 and D1–D5).
//!
//! Every node that the fixtures name carries `.a11y_label("twins.<case>.<part>")`
//! so the measurement harness can match native view frames back to fixture
//! nodes. `TWINS_CASE` selects the app root: `all` (default) stacks every
//! fixture that can live inside a scroll; the `a12.*` cases replace the root
//! so the scroll / safe-area interaction is real.

use std::num::NonZeroUsize;

use waterui::app::App;
use waterui::layout::row as grid_row;
use waterui::layout::HorizontalAlignment;
use waterui::prelude::*;

const ORANGE: &str = "#E67E22";
const BLUE: &str = "#2980B9";
const RED: &str = "#C0392B";

/// Rigid leaf R(w,h): a filled box of exactly `w`×`h`.
fn r(tag: &'static str, w: f32, h: f32) -> impl View {
    Color::srgb_hex(ORANGE).size(w, h).a11y_label(tag)
}

/// Empty leaf E.
fn e(tag: &'static str) -> impl View {
    ().a11y_label(tag)
}

/// Greedy leaf Fill: a color view that stretches to whatever it is given.
fn f(tag: &'static str) -> impl View {
    Color::srgb_hex(BLUE).a11y_label(tag)
}

/// The audit's breakable text: 20 ~10pt monospaced glyphs with one space.
fn t(tag: &'static str) -> impl View {
    text("mmmmmmmmmm mmmmmmmmmm")
        .size(16.7)
        .monospaced()
        .a11y_label(tag)
}

/// The same text capped at `limit` lines.
fn lt(tag: &'static str, limit: usize) -> impl View {
    text("mmmmmmmmmm mmmmmmmmmm")
        .size(16.7)
        .monospaced()
        .line_limit(NonZeroUsize::new(limit).unwrap_or(NonZeroUsize::MIN))
        .a11y_label(tag)
}

/// Short text that fits on one line at any reasonable width.
fn tshort(tag: &'static str) -> impl View {
    text("mm mm mm mm mm mm mm m")
        .size(16.7)
        .monospaced()
        .a11y_label(tag)
}

/// Small monospaced text for the A13 sub-10pt line-height probe.
fn ttiny(tag: &'static str) -> impl View {
    text("aaaaaaaaaa").size(8.4).monospaced().a11y_label(tag)
}

// ---------------------------------------------------------------------------
// A2 — text under a 100×20 proposal, plus the proposal-matrix cells that are
// drivable from public API: finite via `.size(w,h)` clamps, `None` via scroll
// axes. (`∞` and free-form `None` on both axes outside a scroll have no
// public API to inject — those cells are recorded as undrivable.)
// ---------------------------------------------------------------------------
fn a2() -> impl View {
    vstack((
        // wrap-overflow vs truncate: (w=100, h=20)
        t("twins.a2.text")
            .size(100.0, 20.0)
            .a11y_label("twins.a2.wrap")
            .anyview(),
        // line limits at the same proposal
        lt("twins.a2.l1.text", 1)
            .size(100.0, 20.0)
            .a11y_label("twins.a2.l1")
            .anyview(),
        lt("twins.a2.l2.text", 2)
            .size(100.0, 20.0)
            .a11y_label("twins.a2.l2")
            .anyview(),
        // sub-line-height proposal: 8.4pt text under (w=100, h=8)
        t("twins.a2.sub.text")
            .size(100.0, 8.0)
            .a11y_label("twins.a2.subline")
            .anyview(),
        // (w=0, h=20): zero-width proposal
        t("twins.a2.w0.text")
            .size(0.0, 20.0)
            .a11y_label("twins.a2.w0")
            .anyview(),
        // (w=100, h=0): zero-height proposal
        t("twins.a2.h0.text")
            .size(100.0, 0.0)
            .a11y_label("twins.a2.h0")
            .anyview(),
        // short text, (w=100, h=20)
        tshort("twins.a2.short.text")
            .size(100.0, 20.0)
            .a11y_label("twins.a2.short")
            .anyview(),
        // (w=100, h=None): vertical scroll hands content an unspecified height
        scroll(t("twins.a2.inf.text"))
            .size(100.0, 20.0)
            .a11y_label("twins.a2.inf")
            .anyview(),
        // (w=0, h=None): zero-width frame inside a vertical scroll
        scroll(t("twins.a2.w0none.text").width(0.0))
            .size(100.0, 20.0)
            .a11y_label("twins.a2.w0none")
            .anyview(),
        // (w=None, h=20): horizontal scroll hands content an unspecified width
        scroll_horizontal(t("twins.a2.nonew.text").height(20.0))
            .size(100.0, 20.0)
            .a11y_label("twins.a2.nonew")
            .anyview(),
        // (w=None, h=0)
        scroll_horizontal(t("twins.a2.noneh0.text").height(0.0))
            .size(100.0, 20.0)
            .a11y_label("twins.a2.noneh0")
            .anyview(),
        // (w=None, h=None): two-axis scroll
        scroll_both(t("twins.a2.nonenone.text"))
            .size(100.0, 20.0)
            .a11y_label("twins.a2.nonenone")
            .anyview(),
    ))
    .spacing(4.0)
}

// ---------------------------------------------------------------------------
// A3 — H(sp:10)[R(20,10), W{D}, R(20,10)] under a 100×60 proposal (the plain
// member row runs under 100×10). W{} covers: identity member, Frame width 40,
// Padding 10, AspectRatio 2 (fit), ZStack, overlay, background,
// alignment-guide, grid-cell and Absolute; D alternates a real rect and the
// empty view (the `is_empty` slot probe).
// ---------------------------------------------------------------------------
fn a3() -> impl View {
    vstack((a3a().anyview(), a3b().anyview())).spacing(4.0)
}

fn a3a() -> impl View {
    vstack((
        // identity member, empty child — is_empty slot probe
        hstack((
            r("twins.a3.mem.l", 20.0, 10.0),
            e("twins.a3.mem.d"),
            r("twins.a3.mem.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 10.0)
        .a11y_label("twins.a3.mem")
        .anyview(),
        // identity member, rigid child
        hstack((
            r("twins.a3.memr.l", 20.0, 10.0),
            r("twins.a3.memr.d", 20.0, 10.0),
            r("twins.a3.memr.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 10.0)
        .a11y_label("twins.a3.memr")
        .anyview(),
        // Frame width 40 — rigid child
        hstack((
            r("twins.a3.frame.l", 20.0, 10.0),
            r("twins.a3.frame.i", 20.0, 10.0)
                .width(40.0)
                .a11y_label("twins.a3.frame.w"),
            r("twins.a3.frame.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.frame")
        .anyview(),
        // Frame width 40 — empty child
        hstack((
            r("twins.a3.framex.l", 20.0, 10.0),
            e("twins.a3.framex.i")
                .width(40.0)
                .a11y_label("twins.a3.framex.w"),
            r("twins.a3.framex.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.framex")
        .anyview(),
        // Padding 10 — rigid child
        hstack((
            r("twins.a3.pad.l", 20.0, 10.0),
            r("twins.a3.pad.i", 20.0, 10.0)
                .padding_with(10.0)
                .a11y_label("twins.a3.pad.w"),
            r("twins.a3.pad.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.pad")
        .anyview(),
        // Padding 10 — empty child
        hstack((
            r("twins.a3.padx.l", 20.0, 10.0),
            e("twins.a3.padx.i")
                .padding_with(10.0)
                .a11y_label("twins.a3.padx.w"),
            r("twins.a3.padx.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.padx")
        .anyview(),
        // AspectRatio 2 fit — rigid child
        hstack((
            r("twins.a3.aspect.l", 20.0, 10.0),
            aspect_ratio(r("twins.a3.aspect.i", 20.0, 10.0), 2.0)
                .a11y_label("twins.a3.aspect.w"),
            r("twins.a3.aspect.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.aspect")
        .anyview(),
        // AspectRatio 2 fit — empty child
        hstack((
            r("twins.a3.aspectx.l", 20.0, 10.0),
            aspect_ratio(e("twins.a3.aspectx.i"), 2.0)
                .a11y_label("twins.a3.aspectx.w"),
            r("twins.a3.aspectx.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.aspectx")
        .anyview(),
        // second half moved to a3b()
    ))
    .spacing(4.0)
}

fn a3b() -> impl View {
    vstack((
        // ZStack member
        hstack((
            r("twins.a3.zstack.l", 20.0, 10.0),
            zstack((r("twins.a3.zstack.i", 20.0, 10.0),))
                .a11y_label("twins.a3.zstack.w"),
            r("twins.a3.zstack.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.zstack")
        .anyview(),
        // ZStack member — empty child
        hstack((
            r("twins.a3.zstackx.l", 20.0, 10.0),
            zstack((e("twins.a3.zstackx.i"),))
                .a11y_label("twins.a3.zstackx.w"),
            r("twins.a3.zstackx.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.zstackx")
        .anyview(),
        // overlay member
        hstack((
            r("twins.a3.overlay.l", 20.0, 10.0),
            r("twins.a3.overlay.i", 20.0, 10.0)
                .overlay(r("twins.a3.overlay.d", 20.0, 10.0))
                .a11y_label("twins.a3.overlay.w"),
            r("twins.a3.overlay.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.overlay")
        .anyview(),
        // overlay member — empty child
        hstack((
            r("twins.a3.overlayx.l", 20.0, 10.0),
            e("twins.a3.overlayx.i")
                .overlay(r("twins.a3.overlayx.d", 20.0, 10.0))
                .a11y_label("twins.a3.overlayx.w"),
            r("twins.a3.overlayx.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.overlayx")
        .anyview(),
        // background member
        hstack((
            r("twins.a3.bg.l", 20.0, 10.0),
            r("twins.a3.bg.i", 20.0, 10.0)
                .background(r("twins.a3.bg.d", 20.0, 10.0))
                .a11y_label("twins.a3.bg.w"),
            r("twins.a3.bg.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.bg")
        .anyview(),
        // background member — empty child
        hstack((
            r("twins.a3.bgx.l", 20.0, 10.0),
            e("twins.a3.bgx.i")
                .background(r("twins.a3.bgx.d", 20.0, 10.0))
                .a11y_label("twins.a3.bgx.w"),
            r("twins.a3.bgx.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.bgx")
        .anyview(),
        // alignment-guide member: explicit leading guide at -5
        hstack((
            r("twins.a3.guide.l", 20.0, 10.0),
            r("twins.a3.guide.i", 20.0, 10.0)
                .horizontal_alignment_guide(HorizontalAlignment::Leading, |_d| {
                    -5.0
                })
                .a11y_label("twins.a3.guide.w"),
            r("twins.a3.guide.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.guide")
        .anyview(),
        // alignment-guide member — empty child
        hstack((
            r("twins.a3.guidex.l", 20.0, 10.0),
            e("twins.a3.guidex.i")
                .horizontal_alignment_guide(HorizontalAlignment::Leading, |_d| {
                    -5.0
                })
                .a11y_label("twins.a3.guidex.w"),
            r("twins.a3.guidex.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.guidex")
        .anyview(),
        // grid-cell member
        hstack((
            r("twins.a3.grid.l", 20.0, 10.0),
            grid(1, [grid_row((r("twins.a3.grid.i", 20.0, 10.0),))])
                .a11y_label("twins.a3.grid.w"),
            r("twins.a3.grid.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.grid")
        .anyview(),
        // grid-cell member — empty child
        hstack((
            r("twins.a3.gridx.l", 20.0, 10.0),
            grid(1, [grid_row((e("twins.a3.gridx.i"),))])
                .a11y_label("twins.a3.gridx.w"),
            r("twins.a3.gridx.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.gridx")
        .anyview(),
        // absolute member: empty pinned at (0,0) 40×10
        hstack((
            r("twins.a3.abs.l", 20.0, 10.0),
            absolute((e("twins.a3.abs.i").pin(
                PinConstraints::new()
                    .leading(0.0)
                    .top(0.0)
                    .width(40.0)
                    .height(10.0),
            ),))
            .a11y_label("twins.a3.abs.w"),
            r("twins.a3.abs.r", 20.0, 10.0),
        ))
        .spacing(10.0)
        .size(100.0, 60.0)
        .a11y_label("twins.a3.abs")
        .anyview(),
    ))
    .spacing(4.0)
}

// ---------------------------------------------------------------------------
// A11 — scroll containers under a 100×80 host (and the w0 / unspecified-axis
// viewport probes). D4 is the same fixture as the plain vertical scroll.
// ---------------------------------------------------------------------------
fn a11() -> impl View {
    vstack((
        scroll(r("twins.a11.v.content", 60.0, 300.0))
            .size(100.0, 80.0)
            .a11y_label("twins.a11.v")
            .anyview(),
        // cross-axis overflow: content wider than the viewport
        scroll(r("twins.a11.wide.content", 140.0, 300.0))
            .size(100.0, 80.0)
            .a11y_label("twins.a11.wide")
            .anyview(),
        scroll_horizontal(r("twins.a11.h.content", 300.0, 60.0))
            .size(100.0, 80.0)
            .a11y_label("twins.a11.h")
            .anyview(),
        // two-axis scroll, content smaller than the viewport on both axes
        scroll_both(r("twins.a11.both.content", 60.0, 30.0))
            .size(100.0, 80.0)
            .a11y_label("twins.a11.both")
            .anyview(),
        // zero-width viewport proposal
        scroll(r("twins.a11.w0.content", 60.0, 300.0))
            .size(0.0, 80.0)
            .a11y_label("twins.a11.w0")
            .anyview(),
        // inner vertical scroll under an unspecified-width proposal
        scroll_horizontal(
            scroll(r("twins.a11.inf.content", 60.0, 300.0))
                .a11y_label("twins.a11.inf.scroll"),
        )
        .size(100.0, 80.0)
        .a11y_label("twins.a11.inf")
        .anyview(),
    ))
    .spacing(4.0)
}

// ---------------------------------------------------------------------------
// A12 — root-level fixtures; selected by TWINS_CASE because they need the
// window proposal and the real safe-area insets at the app root.
// ---------------------------------------------------------------------------
fn a12_root(case: String) -> impl View {
    match case.as_str() {
        "a12.ignore" => scroll(r("twins.a12.content", 60.0, 300.0))
            .a11y_label("twins.a12.scroll")
            .ignore_safe_area(EdgeSet::ALL)
            .anyview(),
        "a12.bar" => zstack((
            scroll(r("twins.a12.content", 60.0, 300.0))
                .a11y_label("twins.a12.scroll"),
            vstack((
                Color::srgb_hex(RED)
                    .height(44.0)
                    .a11y_label("twins.a12.bar"),
                spacer(),
            )),
        ))
        .anyview(),
        _ => scroll(r("twins.a12.content", 60.0, 300.0))
            .a11y_label("twins.a12.scroll")
            .anyview(),
    }
}

// ---------------------------------------------------------------------------
// A13 — equal-fraction fills and sub-10pt line boxes (scale-2 snapping probes).
// ---------------------------------------------------------------------------
fn a13() -> impl View {
    vstack((
        hstack((
            f("twins.a13.fill.a"),
            f("twins.a13.fill.b"),
            f("twins.a13.fill.c"),
        ))
        .spacing(0.0)
        .size(100.0, 10.0)
        .a11y_label("twins.a13.fills")
        .anyview(),
        vstack((
            ttiny("twins.a13.text.a"),
            ttiny("twins.a13.text.b"),
            ttiny("twins.a13.text.c"),
        ))
        .spacing(0.0)
        .size(100.0, 40.0)
        .a11y_label("twins.a13.texts")
        .anyview(),
    ))
    .spacing(4.0)
}

// ---------------------------------------------------------------------------
// D — the contested divergences. D4 is the a11.v twin and lives there.
// ---------------------------------------------------------------------------
fn d() -> impl View {
    vstack((
        // D1: grid columns at placement — 20-wide vs 80-wide in a 200×50 host
        grid(2, [grid_row((
            r("twins.d1.a", 20.0, 10.0),
            r("twins.d1.b", 80.0, 10.0),
        ))])
        .size(200.0, 50.0)
        .a11y_label("twins.d1")
        .anyview(),
        // D2: AspectRatio(2, fit) around a 30×10 rigid under 100×100
        aspect_ratio(r("twins.d2.i", 30.0, 10.0), 2.0)
            .a11y_label("twins.d2.w")
            .size(100.0, 100.0)
            .a11y_label("twins.d2")
            .anyview(),
        // D3: 20×10 background under a 100×50 base in a 100×50 host
        r("twins.d3.base", 100.0, 50.0)
            .background(r("twins.d3.bg", 20.0, 10.0))
            .a11y_label("twins.d3.w")
            .size(100.0, 50.0)
            .a11y_label("twins.d3")
            .anyview(),
        // D5: ZStack [R(100,100) p0, R(20,20) p1] under 200×200
        zstack((
            r("twins.d5.bottom", 100.0, 100.0),
            r("twins.d5.top", 20.0, 20.0).layout_priority(1),
        ))
        .size(200.0, 200.0)
        .a11y_label("twins.d5")
        .anyview(),
    ))
    .spacing(4.0)
}

fn root_case(case: String) -> impl View {
    match case.as_str() {
        "a12.scroll" | "a12.ignore" | "a12.bar" => a12_root(case).anyview(),
        _ => scroll(
            vstack((
                a2().anyview(),
                a3().anyview(),
                a11().anyview(),
                a13().anyview(),
                d().anyview(),
            ))
            .spacing(8.0),
        )
        .anyview(),
    }
}

pub fn app(env: Environment) -> App {
    let case = std::env::var("TWINS_CASE").unwrap_or_else(|_| "all".to_string());
    App::new(move || root_case(case.clone()), env)
}
