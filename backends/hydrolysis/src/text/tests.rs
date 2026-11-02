//! Engine-neutral contract tests: the behaviours every `TextEngine` must
//! satisfy for the editing and measurement paths that consume it.

use waterui_core::layout::{HorizontalAlignment, Size as LayoutSize, VerticalAlignment};
use waterui_text::font::{Body, FontWeight, ResolvedFont};
use waterui_text::styled::StyledStr;

use super::{
    Affinity, FontFamilyResolution, ParleyEngine, TextEngine, TextLayout, TextPosition,
    TextSelection, TextService, resolve_text_layout_input,
};
use crate::renderer::tests::test_environment;
use crate::renderer::{HydroState, HydrolysisRenderer};

/// Assertions every `TextEngine` must satisfy, run against a [`TextService`]
/// built on that engine.
fn editing_contract<E: TextEngine>(service: &TextService<E>) {
    let env = test_environment();

    // An empty input shapes to zero lines and a zero size.
    let empty =
        resolve_text_layout_input(&StyledStr::plain(""), HorizontalAlignment::Leading, &env);
    let empty_layout = service.shape(&empty, None);
    assert_eq!(empty_layout.line_count(), 0);
    assert_eq!(
        service.dimensions(&empty_layout, None).size,
        LayoutSize::zero()
    );

    let input = resolve_text_layout_input(
        &StyledStr::plain("hello world"),
        HorizontalAlignment::Leading,
        &env,
    );
    let layout = service.shape(&input, None);
    let caret = layout.caret_rect(TextPosition::in_text(8, 11));
    let center = caret.center();

    let word = layout.word_at(
        crate::num_cast::f64_as_f32(center.x),
        crate::num_cast::f64_as_f32(center.y),
    );
    let (word_start, word_end) = (
        word.anchor.index.min(word.focus.index),
        word.anchor.index.max(word.focus.index),
    );
    assert_eq!((word_start, word_end), (6, 11));

    let line = layout.line_at(
        crate::num_cast::f64_as_f32(center.x),
        crate::num_cast::f64_as_f32(center.y),
    );
    let (line_start, line_end) = (
        line.anchor.index.min(line.focus.index),
        line.anchor.index.max(line.focus.index),
    );
    assert_eq!((line_start, line_end), (0, 11));

    // A point just right of the caret's left edge hits the same index.
    let hit = layout.hit_test(
        crate::num_cast::f64_as_f32(caret.x0) + 0.5,
        crate::num_cast::f64_as_f32(center.y),
    );
    assert_eq!(hit.index, 8);

    // Visual moves walk one cluster at a time.
    let zero = TextSelection::collapsed(TextPosition::in_text(0, 11));
    let next = layout.next_visual(zero, false);
    assert_eq!(next.focus.index, 1);
    let extended = layout.next_visual(zero, true);
    assert_eq!(extended.anchor.index, 0);
    assert_eq!(extended.focus.index, 1);
    let previous = layout.previous_visual(zero, false);
    assert_eq!(previous.focus.index, 0);

    // A selection snapped into the middle of a cluster lands on its
    // boundary: in "\u{4e2d}x" byte index 1 sits inside the first,
    // three-byte cluster.
    let mid_character = resolve_text_layout_input(
        &StyledStr::plain("\u{4e2d}x"),
        HorizontalAlignment::Leading,
        &env,
    );
    let mid_character_layout = service.shape(&mid_character, None);
    let snapped =
        mid_character_layout.selection(TextPosition::in_text(1, 4), TextPosition::in_text(1, 4));
    assert_eq!(snapped.anchor.index, 0);
    assert_eq!(snapped.focus.index, 0);

    // Caret positions are grapheme boundaries even where the shaper gives a
    // combining mark a cluster of its own: in "e\u{301}x" byte 1 sits
    // between the letter and its accent, and the grapheme ends at byte 3.
    let combining = resolve_text_layout_input(
        &StyledStr::plain("e\u{301}x"),
        HorizontalAlignment::Leading,
        &env,
    );
    let combining_layout = service.shape(&combining, None);
    let snapped =
        combining_layout.selection(TextPosition::in_text(1, 4), TextPosition::in_text(1, 4));
    assert_eq!((snapped.anchor.index, snapped.focus.index), (0, 0));
    let start = TextSelection::collapsed(TextPosition::in_text(0, 4));
    assert_eq!(combining_layout.next_visual(start, false).focus.index, 3);
    let extended = combining_layout.next_visual(start, true);
    assert_eq!((extended.anchor.index, extended.focus.index), (0, 3));
    let after = TextSelection::collapsed(TextPosition::in_text(3, 4));
    assert_eq!(
        combining_layout.previous_visual(after, false).focus.index,
        0
    );
    let x_end = combining_layout.caret_rect(TextPosition::in_text(4, 4)).x1;
    let y = crate::num_cast::f64_as_f32(combining_layout.caret_rect(after.focus).center().y);
    for step in 0..=20 {
        let x = crate::num_cast::f64_as_f32(x_end * f64::from(step) / 20.0);
        let hit = combining_layout.hit_test(x, y);
        assert_ne!(hit.index, 1, "a hit at x = {x} landed inside the grapheme");
    }

    assert_eq!(TextPosition::in_text(11, 11).affinity, Affinity::Upstream);
}

/// The session's bound engine satisfies the editing contract.
#[test]
fn session_engine_satisfies_the_editing_contract() {
    let service = TextService::new(ParleyEngine::system(FontFamilyResolution::Lenient));
    editing_contract(&service);
}

/// The leaf contract (docs/layout-spec.md §6): the answer is the laid-out
/// width of the truncated line — its kept clusters plus the ellipsis —
/// never the proposal. The cut packs clusters to glyph granularity, so
/// the laid-out line sits within one dropped cluster of the bound.
#[test]
fn a_truncated_leaf_reports_the_drawn_line() {
    let env = test_environment();
    let mut state = HydroState::new(ParleyEngine::system(FontFamilyResolution::Strict));
    let styled = StyledStr::plain("aaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

    let dimensions = HydrolysisRenderer::measure_text_dimensions(
        &mut state,
        styled,
        HorizontalAlignment::Leading,
        &env,
        Some(100.0),
        Some(1),
    );

    assert!(
        dimensions.size.width <= 100.0,
        "the laid-out line stays inside its bound"
    );
    assert!(
        dimensions.size.width > 85.0,
        "the laid-out line fills the bound to within a cluster"
    );
}

/// A resized run spaces on the resized face's own metrics, never the theme
/// slot's absolute pitch: a `.size(40)` text inside a 16/24 `Body` slot
/// must not inherit the default's 24 pt line height — that parked the
/// baseline at 25 pt under a 24 pt frame and cut the glyph tops off at the
/// window edge (water-rs/waterui#2719).
#[test]
fn a_resized_run_spaces_on_its_own_face_metrics() {
    let mut env = test_environment();
    waterui::theme::install_font_signal::<Body>(
        &mut env,
        nami::Computed::constant(
            ResolvedFont::new(16.0, FontWeight::Normal).with_typography_metrics(24.0, 0.0),
        ),
    );
    let mut state = HydroState::new(ParleyEngine::system(FontFamilyResolution::Strict));
    let styled = StyledStr::plain("Hello 0").size(40.0);

    let dimensions = HydrolysisRenderer::measure_text_dimensions(
        &mut state,
        styled,
        HorizontalAlignment::Leading,
        &env,
        Some(600.0),
        None,
    );

    // A Latin face's own pitch at 40 pt exceeds the em — the stale 24 pt
    // the paragraph default carried does not.
    assert!(
        dimensions.size.height >= 40.0,
        "a 40 pt run must report the resized face's own line height, got {}",
        dimensions.size.height
    );
    // Its baseline stays inside the measured frame: the slot pitch left the
    // baseline one point *below* a 24 pt frame, so the glyph tops painted
    // above the window's first row.
    let first_baseline = dimensions.vertical(VerticalAlignment::FirstBaseline);
    assert!(
        first_baseline <= dimensions.size.height,
        "first baseline {first_baseline} must stay inside the measured height {}",
        dimensions.size.height
    );
}

/// A tail the line count alone cuts — the kept line never reached the
/// bound — reports its drawn extent, not the bound.
#[test]
fn a_line_count_truncation_reports_the_drawn_extent() {
    let env = test_environment();
    let mut state = HydroState::new(ParleyEngine::system(FontFamilyResolution::Strict));
    let styled = StyledStr::plain("a\nb\nc");

    let dimensions = HydrolysisRenderer::measure_text_dimensions(
        &mut state,
        styled,
        HorizontalAlignment::Leading,
        &env,
        Some(200.0),
        Some(1),
    );

    assert!(
        dimensions.size.width < 200.0,
        "a bound the text never filled is not reported"
    );
    assert!(dimensions.size.width > 0.0);
}
