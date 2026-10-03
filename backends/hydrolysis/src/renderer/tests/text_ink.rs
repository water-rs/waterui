//! Text measure-versus-paint fidelity on the real font stack.
//!
//! `HeadlessRuntime::new_for_tests` shapes text with the bundled deterministic
//! fonts, which measure snugly; a divergence between the measured advance and
//! the painted ink extent only shows on the fonts the windowed runners load
//! (system collection + `resources/fonts`, including fallback). These tests
//! run the winit runner's font path via
//! [`HeadlessRuntime::new_for_tests_native_fonts`].

use std::time::Instant;

use nami::Binding;
use nami::collection::SignalCollection;
use waterui::ViewExt as _;
use waterui::graphics::color::Srgb;
use waterui::prelude::text;
use waterui::shape::RoundedRectangle;
use waterui::theme::color::Surface;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::stack::{VStack, vstack};
use waterui_text::font::{Body, Font};

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;

const W: u32 = 320;
const H: u32 = 240;

fn rows(rgba8: &[u8]) -> impl Iterator<Item = (usize, &[u8])> {
    rgba8
        .as_chunks::<{ W as usize * 4 }>()
        .0
        .iter()
        .map(<[u8; W as usize * 4]>::as_slice)
        .enumerate()
}

fn px(row: &[u8], x: usize) -> [u8; 3] {
    row[x * 4..x * 4 + 3].try_into().unwrap()
}

/// A pixel carrying any paint at all — white pill or ink, dark glyph ink, or
/// the partial coverage of an antialiased edge. The canvas is solid
/// `#0000FF`, so anything else is something the rasterizer drew.
///
/// Edge detection reads this rather than full whiteness: a pill edge's own
/// AA column and the columns where dark glyph ink overlaps it sit *inside*
/// the capsule's painted bounds, so a first-pure-white-column test lands up
/// to several pixels inside the true edge (while thin twin ink on an
/// antialiased ramp can still hit pure white). Comparing painted extent to
/// painted extent keeps both sides on the same coverage rule.
fn is_paint(px: [u8; 3]) -> bool {
    px != [0, 0, 255]
}

/// A pixel that is neither the white pill/ink nor the blue canvas: glyph ink
/// (solid #1B1B1F or its antialiased blend over the canvas).
fn is_dark_ink(px: [u8; 3]) -> bool {
    px[0] < 0x80 && px[1] < 0x80 && px[2] < 0xB4
}

/// Painted bounding box of the capsule. A pill row carries the capsule edge
/// to edge as one contiguous painted run — interior glyph ink counts as
/// paint, so it cannot fragment the run the way a white-pixel scan did —
/// while a twin-ink row only scatters thin strokes, so the wide run selects
/// pill rows alone.
fn capsule_bounds(rgba8: &[u8]) -> Option<(usize, usize, usize, usize)> {
    let mut x0 = usize::MAX;
    let mut x1 = 0usize;
    let mut y0 = usize::MAX;
    let mut y1 = 0usize;
    for (y, row) in rows(rgba8) {
        let mut start = 0usize;
        let mut in_run = false;
        let mut widest = (0usize, 0usize);
        for x in 0..=W as usize {
            let painted = x < W as usize && is_paint(px(row, x));
            if painted && !in_run {
                start = x;
                in_run = true;
            } else if !painted && in_run {
                in_run = false;
                if x - start > widest.1 - widest.0 {
                    widest = (start, x);
                }
            }
        }
        if widest.1 - widest.0 >= 40 {
            x0 = x0.min(widest.0);
            x1 = x1.max(widest.1 - 1);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
    }
    (x0 <= x1 && y0 <= y1).then_some((x0, x1, y0, y1))
}

fn row_span<F: Fn([u8; 3]) -> bool>(
    rgba8: &[u8],
    y_band: core::ops::Range<usize>,
    pred: F,
) -> Option<(usize, usize, usize)> {
    let mut x0 = usize::MAX;
    let mut x1 = 0usize;
    let mut n = 0usize;
    for (y, row) in rows(rgba8) {
        if !y_band.contains(&y) {
            continue;
        }
        for x in 0..W as usize {
            if pred(px(row, x)) {
                x0 = x0.min(x);
                x1 = x1.max(x);
                n += 1;
            }
        }
    }
    (n > 0).then_some((x0, x1, n))
}

/// Issue #237: a capsule-clipped text stack must not paint ink outside the
/// frame its measure produced. The clip shape can hide overflow (it trims to
/// exactly the measured bounds), so the check pairs each clipped pill with an
/// *unclipped* twin carrying the same rows in white ink on the canvas: the
/// twin's ink span is the true painted extent and must fit inside the pill's
/// measured bounds.
///
/// `padding_with((4.0, 0.0))` gives the capsule exactly the measured advance
/// horizontally, so `ink ⊆ capsule` is precisely `ink ⊆ advance`.
#[test]
fn painted_ink_stays_within_measured_frame_native_fonts() {
    let builder = AnyViewBuilder::<AnyView>::new(|| {
        let texts = [
            "suggestion 1",
            "WAVy jiggle",
            "clipped pill row",
            "suggestion 4",
        ];
        let pill = VStack::for_each(
            SignalCollection::new(Binding::container(
                (0..texts.len() as u64).map(SelfId::new).collect::<Vec<_>>(),
            )),
            move |id: SelfId<u64>| text(texts[id.into_inner() as usize]).body(),
        )
        .spacing(0.0)
        .padding_with((4.0, 0.0))
        .foreground(Srgb::from_hex("#1B1B1F"))
        .background(Surface)
        .clip(RoundedRectangle::new(6.0));
        // Unclipped twin in white ink, same text and padding: its painted ink
        // extent is directly visible on the blue canvas.
        let twin = VStack::for_each(
            SignalCollection::new(Binding::container(
                (0..texts.len() as u64).map(SelfId::new).collect::<Vec<_>>(),
            )),
            move |id: SelfId<u64>| text(texts[id.into_inner() as usize]).body(),
        )
        .spacing(0.0)
        .padding_with((4.0, 0.0))
        .foreground(Srgb::from_hex("#FFFFFF"));
        AnyView::new(vstack((pill, twin)).background(Srgb::from_hex("#0000FF")))
    });

    let mut runtime = HeadlessRuntime::new_for_tests_native_fonts(
        test_environment(),
        builder,
        W,
        H,
        MinimalTestTheme::default(),
    );
    let at = Instant::now();
    for _ in 0..4 {
        runtime.pump_at(false, at);
    }
    let snapshot = runtime.pump_at(true, at).snapshot.expect("capture");

    let (cx0, cx1, cy0, cy1) = capsule_bounds(&snapshot.rgba8).expect("capsule pill painted");
    let capsule_w = cx1 - cx0 + 1;
    let capsule_h = cy1 - cy0 + 1;
    // Four ~19 pt rows + 8 pt vertical padding: proves the fonts loaded and the
    // rows laid out at their measured heights.
    assert!(
        capsule_w >= 60 && capsule_h >= 60,
        "capsule should wrap all four rows, got {capsule_w}x{capsule_h}"
    );

    // Unclipped twin ink: painted pixels strictly below the capsule.
    let (ix0, ix1, ink_px) = row_span(&snapshot.rgba8, cy1 + 4..H as usize, is_paint)
        .expect("twin ink painted — native fonts must render the rows");
    assert!(
        ink_px > 200,
        "twin ink too sparse ({ink_px}px): fonts may not have loaded"
    );

    // The invariant: the painted ink extent fits inside the frame the
    // measure produced (the capsule). Both extents are detected at the same
    // coverage granularity — any painted pixel — so a column of ink the
    // capsule never reached is a real escape, not an AA rounding edge.
    assert!(
        ix0 >= cx0 && ix1 <= cx1,
        "twin ink x {ix0}..={ix1} escapes measured frame x {cx0}..={cx1}"
    );

    // No ink-colored pixel outside the capsule's measured bounds inside its
    // neighborhood either — ink clipped by the stadium caps may not paint, but
    // none may overpaint the frame.
    let mut escaped = 0usize;
    for (y, row) in rows(&snapshot.rgba8) {
        if y + 4 < cy0 || y > cy1 + 4 {
            continue;
        }
        for x in 0..W as usize {
            let p = px(row, x);
            if is_dark_ink(p) && (x + 1 < cx0 || x > cx1 + 1) {
                escaped += 1;
            }
        }
    }
    assert_eq!(escaped, 0, "glyph ink painted outside the measured frame");
}

/// Same invariant for a single plain `Text` — no collection, no clipping
/// container beyond the capsule itself.
#[test]
fn painted_ink_stays_within_measured_frame_single_text() {
    let builder = AnyViewBuilder::<AnyView>::new(|| {
        AnyView::new(
            vstack((
                text("clipped pill row")
                    .body()
                    .foreground(Srgb::from_hex("#1B1B1F"))
                    .background(Surface)
                    .clip(RoundedRectangle::new(6.0)),
                text("clipped pill row")
                    .body()
                    .foreground(Srgb::from_hex("#FFFFFF")),
            ))
            .background(Srgb::from_hex("#0000FF")),
        )
    });

    let mut runtime = HeadlessRuntime::new_for_tests_native_fonts(
        test_environment(),
        builder,
        W,
        H,
        MinimalTestTheme::default(),
    );
    let at = Instant::now();
    for _ in 0..4 {
        runtime.pump_at(false, at);
    }
    let snapshot = runtime.pump_at(true, at).snapshot.expect("capture");

    let (cx0, cx1, _cy0, cy1) = capsule_bounds(&snapshot.rgba8).expect("capsule painted");
    let (ix0, ix1, _) =
        row_span(&snapshot.rgba8, cy1 + 2..H as usize, is_paint).expect("twin ink painted");
    assert!(
        ix0 >= cx0 && ix1 <= cx1,
        "twin ink x {ix0}..={ix1} escapes measured frame x {cx0}..={cx1}"
    );
}

/// Same invariant on a face whose outlines genuinely overhang the pen
/// advance — the bundled Pacifico subset ('p' starts 0.119em left of the
/// pen origin, 'y'/'f' end ~0.10em right of the advance) — so the check
/// exercises real overhang on every host, not only where the platform's
/// default face happens to overhang.
#[test]
fn painted_ink_stays_within_measured_frame_overhang_font() {
    let builder = AnyViewBuilder::<AnyView>::new(|| {
        AnyView::new(
            vstack((
                text("play fully")
                    .body()
                    .font(Font::new(Body).family("Pacifico"))
                    .foreground(Srgb::from_hex("#1B1B1F"))
                    .background(Surface)
                    .clip(RoundedRectangle::new(6.0)),
                text("play fully")
                    .body()
                    .font(Font::new(Body).family("Pacifico"))
                    .foreground(Srgb::from_hex("#FFFFFF")),
            ))
            .background(Srgb::from_hex("#0000FF")),
        )
    });

    let mut runtime = HeadlessRuntime::new_for_tests_native_fonts(
        test_environment(),
        builder,
        W,
        H,
        MinimalTestTheme::default(),
    );
    let at = Instant::now();
    for _ in 0..4 {
        runtime.pump_at(false, at);
    }
    let snapshot = runtime.pump_at(true, at).snapshot.expect("capture");

    let (cx0, cx1, _cy0, cy1) = capsule_bounds(&snapshot.rgba8).expect("capsule painted");
    let (ix0, ix1, _) =
        row_span(&snapshot.rgba8, cy1 + 2..H as usize, is_paint).expect("twin ink painted");
    assert!(
        ix0 >= cx0 && ix1 <= cx1,
        "twin ink x {ix0}..={ix1} escapes measured frame x {cx0}..={cx1}"
    );
}

/// `GlyphMetrics::bounds` must agree with drawing the outline through
/// `ControlBoundsPen` — the ink-extent measure substitutes the former (the
/// `glyf` header bounds) for the latter, so they must report the same extents
/// on a TrueType face with real overhang.
#[test]
fn glyph_bounds_fast_path_agrees_with_drawn_outline() {
    use skrifa::instance::{LocationRef, Size};
    use skrifa::outline::{DrawSettings, pen::ControlBoundsPen};
    use skrifa::{FontRef, GlyphId, MetadataProvider};

    let font = FontRef::new(include_bytes!("../../../test-fonts/PacificoSubset.ttf"))
        .expect("bundled Pacifico subset parses");
    // Both paths read the same font-unit extents; at a scaled size the header
    // path rounds through FreeType's 16.16 fixed-point scale while the pen
    // scales in float, so allow one font unit of rounding in pixels.
    let location = LocationRef::default();
    for size in [Size::unscaled(), Size::new(16.0)] {
        let slack = size.ppem().map_or(1e-3, |ppem| {
            ppem / f32::from(font.metrics(size, location).units_per_em) + 1e-3
        });
        let metrics = font.glyph_metrics(size, location);
        let outlines = font.outline_glyphs();
        let glyph_count = font.metrics(size, location).glyph_count;
        for glyph_id in 0..glyph_count {
            let glyph = GlyphId::new(u32::from(glyph_id));
            let header = metrics.bounds(glyph);
            let drawn = outlines.get(glyph).and_then(|outline| {
                let mut pen = ControlBoundsPen::new();
                outline
                    .draw(DrawSettings::unhinted(size, location), &mut pen)
                    .ok()?;
                pen.bounding_box()
            });
            let (Some(header), Some(drawn)) = (header, drawn) else {
                // A glyph with no drawn outline reports no ink either way.
                continue;
            };
            assert!(
                (header.x_min - drawn.x_min).abs() <= slack
                    && (header.x_max - drawn.x_max).abs() <= slack,
                "glyph {glyph_id} @ {size:?}: header x {}..={} vs drawn {}..={}",
                header.x_min,
                header.x_max,
                drawn.x_min,
                drawn.x_max
            );
        }
    }
}
