//! Engine tests: the moved shaping, truncation, family and glyph-bounds
//! suites, each running against the parley engine directly.

use super::*;
use crate::renderer::tests::test_environment;
use crate::text::{TextService, resolve_text_layout_input};
use waterui_core::Environment;
use waterui_text::styled::StyledStr;

fn test_input(env: &Environment, text: &'static str) -> ResolvedTextLayoutInput {
    resolve_text_layout_input(&StyledStr::plain(text), HorizontalAlignment::Leading, env)
}

#[test]
fn explicit_family_list_is_preserved_for_parley_css_parsing() {
    let family = font_family(Some("Roboto, Noto Sans CJK SC, sans-serif"));

    assert_eq!(
        family,
        parley::FontFamily::Source("Roboto, Noto Sans CJK SC, sans-serif".into())
    );
}

#[test]
fn missing_family_uses_sans_serif_generic() {
    assert_eq!(
        font_family(None),
        parley::FontFamily::from(parley::style::GenericFamily::SansSerif)
    );
}

#[test]
fn logical_text_alignment_follows_environment_direction() {
    assert_eq!(
        parley_alignment(HorizontalAlignment::Leading, false),
        parley::Alignment::Left
    );
    assert_eq!(
        parley_alignment(HorizontalAlignment::Leading, true),
        parley::Alignment::Right
    );
    assert_eq!(
        parley_alignment(HorizontalAlignment::Trailing, true),
        parley::Alignment::Left
    );
}

/// The logically-last cluster's character on a line — where the marker
/// sits after tail truncation.
fn last_cluster_char(line: &parley::Line<'_, [u8; 4]>) -> Option<char> {
    let mut tail = None;
    for run in line.runs() {
        for cluster in run.clusters() {
            tail = Some(cluster.source_char());
        }
    }
    tail
}

#[test]
fn a_truncated_line_ends_in_an_ellipsis_inside_its_bound() {
    let env = test_environment();
    let service = TextService::new(ParleyEngine::system(FontFamilyResolution::Strict));
    let input = test_input(
        &env,
        "a preview long enough that a single line cannot hold it",
    );

    let layout = service.shape_limited(&input, Some(60.0), Some(1));

    assert_eq!(
        layout.0.layout.len(),
        1,
        "a one-line limit lays out one line"
    );
    let line = layout.0.layout.get(0).expect("one line");
    assert!(
        line.metrics().advance <= 60.0,
        "the truncated line stays inside its bound"
    );
    assert!(
        line.metrics().advance > 45.0,
        "the cut fills the bound to glyph granularity"
    );
    assert_eq!(
        last_cluster_char(&line),
        Some(TAIL_ELLIPSIS),
        "the drawn line ends in an ellipsis"
    );
}

#[test]
fn a_multiline_limit_carries_the_ellipsis_on_the_last_line() {
    let env = test_environment();
    let service = TextService::new(ParleyEngine::system(FontFamilyResolution::Strict));
    let input = test_input(
        &env,
        "a preview long enough that it wraps well past the two lines it may show",
    );

    let layout = service.shape_limited(&input, Some(80.0), Some(2));

    assert_eq!(
        layout.0.layout.len(),
        2,
        "the truncated text keeps its allowed lines"
    );
    let last = layout.0.layout.get(1).expect("last line");
    assert!(last.metrics().advance <= 80.0);
    assert_eq!(last_cluster_char(&last), Some(TAIL_ELLIPSIS));
    let first = layout.0.layout.get(0).expect("first line");
    assert_ne!(
        last_cluster_char(&first),
        Some(TAIL_ELLIPSIS),
        "only the last line carries the marker"
    );
}

#[test]
fn a_text_that_fits_its_limit_is_not_truncated() {
    let env = test_environment();
    let service = TextService::new(ParleyEngine::system(FontFamilyResolution::Strict));
    let input = test_input(&env, "short");

    let layout = service.shape_limited(&input, Some(200.0), Some(1));

    let line = layout.0.layout.get(0).expect("one line");
    assert_eq!(last_cluster_char(&line), Some('t'));
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

    // The installed Pacifico face, found through the font collection by
    // family name — the same lookup the strict family-resolution gate makes.
    let mut font_cx = parley::FontContext::new();
    let family = font_cx.collection.family_by_name("Pacifico").expect(
        "font family `Pacifico` is not installed; install the test fonts with \
         `uv run backends/hydrolysis/test-fonts/install.py`",
    );
    let face = family
        .fonts()
        .first()
        .expect("the installed Pacifico family carries a face");
    let index = face.index();
    let blob = face
        .load(Some(&mut font_cx.source_cache))
        .expect("the installed Pacifico face loads");
    let font = FontRef::from_index(blob.data(), index).expect("installed Pacifico face parses");
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
