//! Engine tests: the moved shaping, truncation, family and glyph-bounds
//! suites, each running against the parley engine directly.

use parley::fontique::{Blob, Collection, CollectionOptions};

use super::*;
use crate::renderer::tests::test_environment;
use crate::text::{TextService, resolve_text_layout_input};
use waterui_core::Environment;
use waterui_text::styled::StyledStr;

fn test_input(env: &Environment, text: &'static str) -> ResolvedTextLayoutInput {
    resolve_text_layout_input(&StyledStr::plain(text), HorizontalAlignment::Leading, env)
}

#[test]
fn declared_noto_sans_draws_missing_latin_marks_without_replacing_roboto() {
    let roboto: Arc<[u8]> = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-fonts/Roboto-Web-Variable.ttf"),
    )
    .expect("the test-fonts setup generates the real web Roboto face")
    .into();
    let noto = fonts::installed_font_bytes("Noto Sans");
    assert!(
        skrifa::FontRef::from_index(&noto, 0)
            .expect("Noto Sans parses")
            .charmap()
            .map('\u{0350}')
            .is_some(),
        "the fallback face must really cover U+0350"
    );
    assert!(
        skrifa::FontRef::from_index(&roboto, 0)
            .expect("web Roboto parses")
            .charmap()
            .map('\u{0350}')
            .is_none(),
        "the primary web face must really need fallback for U+0350"
    );
    let mut font_cx = parley::FontContext {
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
        source_cache: parley::fontique::SourceCache::default(),
    };
    let mut families = fonts::ResourceFontFamilies::default();
    for (name, data) in [("Roboto", &roboto), ("Noto Sans", &noto)] {
        let registered = font_cx
            .collection
            .register_fonts(Blob::new(Arc::new(data.to_vec())), None);
        assert!(!registered.is_empty());
        families.classify(name, &registered);
    }
    families.install(&mut font_cx.collection);
    let engine = ParleyEngine::new(font_cx, FontFamilyResolution::Strict);
    let env = test_environment();
    let plain = engine.shape(&test_input(&env, "Z"), None);
    let run = plain
        .0
        .layout
        .get(0)
        .expect("Latin line")
        .runs()
        .next()
        .expect("Latin run");
    assert!(
        run.font().data.as_ref() == roboto.as_ref(),
        "plain Latin must retain the primary Roboto face"
    );

    let combined = engine.shape(&test_input(&env, "Z\u{0350}"), None);
    let line = combined.0.layout.get(0).expect("combining-mark line");
    assert!(
        line.runs()
            .any(|run| run.font().data.as_ref() == noto.as_ref())
    );
    for run in line.runs() {
        for cluster in run.clusters() {
            assert!(cluster.glyphs().all(|glyph| glyph.id != 0));
        }
    }
}

#[test]
fn recorded_glyph_positions_preserve_parley_mark_offsets_and_fragment_shift() {
    let roboto: Arc<[u8]> = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-fonts/Roboto-Regular.ttf"),
    )
    .expect("the test-fonts setup generates the pinned Roboto Regular face")
    .into();
    let mut font_cx = parley::FontContext {
        collection: parley::fontique::Collection::new(parley::fontique::CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
        source_cache: parley::fontique::SourceCache::default(),
    };
    let registered = font_cx
        .collection
        .register_fonts(parley::fontique::Blob::new(Arc::new(roboto.to_vec())), None);
    assert!(!registered.is_empty());
    let mut families = fonts::ResourceFontFamilies::default();
    families.classify("Roboto", &registered);
    families.install(&mut font_cx.collection);
    let engine = ParleyEngine::new(font_cx, FontFamilyResolution::Strict);
    let env = test_environment();
    let layout = engine.shape(&test_input(&env, "Z\u{0350}"), None);
    let line = layout.0.layout.get(0).expect("combining-mark line");
    assert!(
        line.items().any(|item| {
            if let parley::PositionedLayoutItem::GlyphRun(run) = item {
                run.glyphs().any(|glyph| glyph.y.abs() > 0.0)
            } else {
                false
            }
        }),
        "the real fixture must exercise a nonzero vertical shaping offset"
    );

    for x_shift in [0.0, 13.0] {
        let expected: Vec<_> = line
            .items()
            .filter_map(|item| match item {
                parley::PositionedLayoutItem::GlyphRun(run) => Some(run),
                parley::PositionedLayoutItem::InlineBox(_) => None,
            })
            .flat_map(|run| {
                run.positioned_glyphs()
                    .map(|glyph| (glyph.id, (glyph.x + x_shift).to_bits(), glyph.y.to_bits()))
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut scene = Recording::new();
        let mut counters = FrameWorkCounters::default();
        encode_text_layout(&mut scene, &mut counters, &layout, None, x_shift, None);
        let recorded: Vec<_> = scene
            .glyph_runs()
            .flat_map(|(_, glyphs)| glyphs)
            .map(|glyph| (glyph.id, glyph.x.to_bits(), glyph.y.to_bits()))
            .collect();
        assert_eq!(recorded, expected, "fragment shift {x_shift}");
    }
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
fn missing_family_uses_system_ui_generic() {
    assert_eq!(
        font_family(None),
        parley::FontFamily::from(parley::style::GenericFamily::SystemUi)
    );
    assert_eq!(
        emoji_font_family(None),
        parley::FontFamily::List(std::borrow::Cow::Owned(vec![
            parley::FontFamilyName::Generic(parley::GenericFamily::Emoji),
            parley::FontFamilyName::Generic(parley::GenericFamily::SystemUi),
        ]))
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
fn last_cluster_char(line: &parley::Line<'_, super::RunBrush>, text: &str) -> Option<char> {
    let mut tail = None;
    for run in line.runs() {
        for cluster in run.clusters() {
            tail = text[cluster.text_range()].chars().next();
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
        last_cluster_char(&line, &layout.0.text),
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
    assert_eq!(
        last_cluster_char(&last, &layout.0.text),
        Some(TAIL_ELLIPSIS)
    );
    let first = layout.0.layout.get(0).expect("first line");
    assert_ne!(
        last_cluster_char(&first, &layout.0.text),
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
    assert_eq!(last_cluster_char(&line, &layout.0.text), Some('t'));
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

fn first_run_metrics(layout: &ParleyLayout) -> (parley::FontData, Vec<i16>, f32) {
    let line = layout.0.layout.get(0).expect("shaped line");
    let run = line.runs().next().expect("shaped run");
    let font = run.font().clone();
    let coords: Vec<i16> = run
        .normalized_coords()
        .iter()
        .map(|coord| coord.to_bits())
        .collect();
    let advance = line.metrics().advance;
    (font, coords, advance)
}

fn shape_styled_system_text(
    engine: &ParleyEngine,
    env: &Environment,
    content: &'static str,
    size: f32,
    weight: waterui_text::font::FontWeight,
) -> ParleyLayout {
    use waterui_text::font::Font;
    use waterui_text::styled::Style;

    let mut styled = StyledStr::empty();
    styled.push(
        content,
        Style::default().font(Font::default().font_size(size).weight(weight)),
    );
    let input = resolve_text_layout_input(&styled, HorizontalAlignment::Leading, env);
    engine.shape(&input, None)
}

#[test]
fn optical_size_variation_tracks_font_size() {
    use skrifa::raw::types::Tag as SkrifaTag;
    use skrifa::{FontRef, MetadataProvider as _};
    use waterui_text::font::FontWeight;

    let env = test_environment();
    let engine = ParleyEngine::system(FontFamilyResolution::Strict);

    let regular_17 = shape_styled_system_text(&engine, &env, "Continue", 17.0, FontWeight::Normal);
    let regular_28 = shape_styled_system_text(&engine, &env, "Continue", 28.0, FontWeight::Normal);
    let regular_48 = shape_styled_system_text(&engine, &env, "Continue", 48.0, FontWeight::Normal);
    let mut default_input = test_input(&env, "Continue");
    default_input.default_font.size = 17.0;
    default_input.spans.clear();
    let default_17 = engine.shape(&default_input, None);

    let (font_17, coords_17, advance_17) = first_run_metrics(&regular_17);
    let (font_28, coords_28, _) = first_run_metrics(&regular_28);
    let (font_48, coords_48, _) = first_run_metrics(&regular_48);
    let (_, coords_default_17, advance_default_17) = first_run_metrics(&default_17);

    let parsed_17 =
        FontRef::from_index(font_17.data.as_ref(), font_17.index).expect("system font parses");
    if let Some((opsz_index, _)) = parsed_17
        .axes()
        .iter()
        .enumerate()
        .find(|(_, axis)| axis.tag() == SkrifaTag::new(b"opsz"))
    {
        let expected_opsz = |font: &parley::FontData, size: f32| {
            let parsed =
                FontRef::from_index(font.data.as_ref(), font.index).expect("system font parses");
            parsed
                .axes()
                .location([(SkrifaTag::new(b"opsz"), size)])
                .coords()[opsz_index]
                .to_bits()
        };
        let expected_17 = expected_opsz(&font_17, 17.0);
        let expected_28 = expected_opsz(&font_28, 28.0);
        let expected_48 = expected_opsz(&font_48, 48.0);
        // Parley omits normalized coordinates when every axis sits at its default (0).
        assert_eq!(
            coords_17.get(opsz_index).copied().unwrap_or(0),
            expected_17,
            "17 pt run must set opsz variation coordinate to 17.0"
        );
        assert_eq!(
            coords_default_17.get(opsz_index).copied().unwrap_or(0),
            expected_17,
            "17 pt default-font run must set opsz variation coordinate to 17.0"
        );
        assert_eq!(
            coords_28.get(opsz_index).copied().unwrap_or(0),
            expected_28,
            "28 pt span must set opsz variation coordinate to 28.0"
        );
        assert_eq!(
            coords_48.get(opsz_index).copied().unwrap_or(0),
            expected_48,
            "48 pt span must set opsz variation coordinate to 48.0"
        );
        assert_ne!(
            expected_17, expected_28,
            "variable opsz axis must distinguish 17 pt and 28 pt optical sizes"
        );
        assert_ne!(
            expected_28, expected_48,
            "variable opsz axis must distinguish 28 pt and 48 pt optical sizes"
        );
    }

    // On macOS, `.SF NS` defines opsz 17..96 (default 28); shaping 17 pt
    // Regular "Continue" at opsz=17 yields ~137.21 px at 2x (vs ~122.86 px
    // when opsz falls back to the 28 pt display master).
    #[cfg(target_os = "macos")]
    {
        let scaled_advance = advance_17 * 2.0;
        assert!(
            (scaled_advance - 137.21).abs() < 1.5,
            "17 pt SystemUi \"Continue\" advance at 2x should match native optical size (~137.21 px), got {scaled_advance}"
        );
        assert!(
            (advance_default_17 - advance_17).abs() < 1e-3,
            "default-font and span-styled 17 pt runs should share identical optical-size advance ({advance_default_17} vs {advance_17})"
        );
    }
    let _ = (advance_17, advance_default_17);
}

#[test]
fn optical_size_variation_preserves_font_weight() {
    use skrifa::raw::types::Tag as SkrifaTag;
    use skrifa::{FontRef, MetadataProvider as _};
    use waterui_text::font::FontWeight;

    let env = test_environment();
    let engine = ParleyEngine::system(FontFamilyResolution::Strict);

    let semibold_17 = shape_styled_system_text(&engine, &env, "Root", 17.0, FontWeight::SemiBold);
    let regular_17 = shape_styled_system_text(&engine, &env, "Root", 17.0, FontWeight::Normal);

    let (font_semi, coords_semi, _) = first_run_metrics(&semibold_17);
    let (font_reg, coords_reg, _) = first_run_metrics(&regular_17);

    let parsed_semi = FontRef::from_index(font_semi.data.as_ref(), font_semi.index)
        .expect("semibold system font parses");
    if font_semi.data.as_ref() == font_reg.data.as_ref()
        && font_semi.index == font_reg.index
        && let Some((wght_index, _)) = parsed_semi
            .axes()
            .iter()
            .enumerate()
            .find(|(_, axis)| axis.tag() == SkrifaTag::new(b"wght"))
    {
        assert_ne!(
            coords_semi.get(wght_index),
            coords_reg.get(wght_index),
            "setting opsz FontVariations must preserve distinct FontWeight wght coordinates"
        );
    } else {
        assert!(
            font_semi.data.as_ref() != font_reg.data.as_ref() || font_semi.index != font_reg.index,
            "Semibold must select a distinct face or wght coordinate from Normal"
        );
    }
}
