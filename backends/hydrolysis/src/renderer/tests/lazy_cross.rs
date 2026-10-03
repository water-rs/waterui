//! Cross-axis contract for water-rs/waterui#1232: a `VStack::for_each`
//! measured through the view path (`measure_view_dimensions_with_proposal` →
//! `Native<LazyContainer>::dimensions`) must answer what its item answers
//! under the same proposal — the retained `LazyStackNode` already measures
//! the sample item under `(cross, None)`, so the two paths must agree.

use super::*;
use crate::renderer::normalize_layout_view;
use waterui::prelude::*;
use waterui_core::id::SelfId;

type RowFactory = fn() -> AnyView;

fn row_one_line() -> AnyView {
    AnyView::new(hstack((text("one line"), spacer(), text("x"))))
}
fn row_multi_line() -> AnyView {
    AnyView::new(hstack((text("a\nb"), spacer(), text("x"))))
}
fn row_nested_vstack() -> AnyView {
    AnyView::new(hstack((
        vstack((text("a"), text("b"))),
        spacer(),
        text("x"),
    )))
}
fn row_dash_2child() -> AnyView {
    AnyView::new(hstack((text("a — b"), spacer())))
}
fn row_vstack_2child() -> AnyView {
    AnyView::new(hstack((vstack((text("a"), text("b"))), spacer())))
}
// A row whose intrinsic width exceeds the proposal: the wrapped content
// still fits, so the proposal-aware answer must be the proposal, not the
// intrinsic width.
fn row_long_wrap() -> AnyView {
    AnyView::new(hstack((
        text("a considerably longer line of text that wraps under 308"),
        spacer(),
        text("x"),
    )))
}

#[test]
fn lazy_view_path_answers_what_its_item_answers() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let mut state = HydroState::default();

    let shapes: [(&str, RowFactory); 6] = [
        ("one-line", row_one_line),
        ("multi-line", row_multi_line),
        ("nested-vstack", row_nested_vstack),
        ("dash-2child", row_dash_2child),
        ("vstack-2child", row_vstack_2child),
        ("long-wrap", row_long_wrap),
    ];

    let mut mismatches = Vec::new();
    for (name, make_row) in shapes {
        let proposal = ProposalSize::new(Some(308.0), None);
        let lazy = normalize_layout_view(
            AnyView::new(VStack::for_each(
                (0..40).map(SelfId::new).collect::<Vec<_>>(),
                move |_| make_row(),
            )),
            &env,
        );
        let lazy_answer =
            measure_view_dimensions_with_proposal(&lazy, proposal, &mut state, &env, &theme).size;
        let row_view = normalize_layout_view(make_row(), &env);
        let item_answer =
            measure_view_dimensions_with_proposal(&row_view, proposal, &mut state, &env, &theme)
                .size;
        if lazy_answer.width != item_answer.width {
            mismatches.push(format!(
                "{name}: lazy {lazy_answer:?} vs item {item_answer:?}"
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "lazy stack answered differently from its items under (308, None):\n{}",
        mismatches.join("\n")
    );
}

/// The issue's shell through the same view path: a `scroll` wrapping the
/// lazy stack must report the offered cross extent — the content's intrinsic
/// is what the viewport clips, not what the scroll is.
#[test]
fn scroll_shell_answers_its_proposal() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let mut state = HydroState::default();
    let shell = normalize_layout_view(
        AnyView::new(scroll(vstack((VStack::for_each(
            (0..40).map(SelfId::new).collect::<Vec<_>>(),
            move |_| row_long_wrap(),
        ),)))),
        &env,
    );
    let answer = measure_view_dimensions_with_proposal(
        &shell,
        ProposalSize::new(Some(308.0), None),
        &mut state,
        &env,
        &theme,
    )
    .size;
    assert_eq!(
        answer.width, 308.0,
        "scroll wrapping a lazy stack answered {answer:?} under (308, None)"
    );
}
