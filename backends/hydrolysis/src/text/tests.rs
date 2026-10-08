//! Engine-neutral contract tests: the behaviours every `TextEngine` must
//! satisfy for the editing and measurement paths that consume it.

use waterui_core::layout::{HorizontalAlignment, Size as LayoutSize};
use waterui_text::styled::StyledStr;

use super::{
    Affinity, FontFamilyResolution, SessionTextEngine, TextEngine, TextLayout, TextPosition,
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
    // three-byte cluster. (Parley splits "e\u{301}x" into one cluster per
    // scalar — byte 1 is already a boundary there — so this probes a
    // multi-byte scalar instead.)
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

    assert_eq!(TextPosition::in_text(11, 11).affinity, Affinity::Upstream);
}

/// The session's bound engine satisfies the editing contract.
#[test]
fn session_engine_satisfies_the_editing_contract() {
    let service = TextService::new(SessionTextEngine::system(FontFamilyResolution::Lenient));
    editing_contract(&service);
}

/// The leaf contract (docs/layout-spec.md §6): the answer is the laid-out
/// width of the truncated line — its kept clusters plus the ellipsis —
/// never the proposal. The cut packs clusters to glyph granularity, so
/// the laid-out line sits within one dropped cluster of the bound.
#[test]
fn a_truncated_leaf_reports_the_drawn_line() {
    let env = test_environment();
    let mut state = HydroState::new(SessionTextEngine::system(FontFamilyResolution::Strict));
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

/// A tail the line count alone cuts — the kept line never reached the
/// bound — reports its drawn extent, not the bound.
#[test]
fn a_line_count_truncation_reports_the_drawn_extent() {
    let env = test_environment();
    let mut state = HydroState::new(SessionTextEngine::system(FontFamilyResolution::Strict));
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
