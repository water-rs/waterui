//! Cross-axis contract for water-rs/waterui#1232: a realized
//! `VStack::for_each` must answer what its item answers under the same
//! proposal — the `LazyStackNode` measures its first item under
//! `(cross, None)`.

use super::*;

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
// A row whose members are sized so their f32 sum under the 308 proposal is
// not exactly 308 (`106.69205 + spacer + 5.2527266 + 20` narrows to
// `307.99997`), so the comparison covers the inexact case on every machine.
fn row_inexact_sum() -> AnyView {
    AnyView::new(hstack((
        ().size(106.69205, 10.0),
        spacer(),
        ().size(5.252_726_6, 10.0),
    )))
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
#[expect(
    clippy::float_cmp,
    reason = "the comparison is exact by design: the lazy stack's cross answer and the item's answer come from the same measurement of the same item, so they must be bit-identical"
)]
fn a_lazy_stack_answers_what_its_item_answers() {
    use waterui::layout::scroll::Axis as ScrollAxis;

    let env = test_environment().extending(ScrollAxis::Vertical);
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());

    let shapes: [(&str, RowFactory); 7] = [
        ("one-line", row_one_line),
        ("multi-line", row_multi_line),
        ("nested-vstack", row_nested_vstack),
        ("dash-2child", row_dash_2child),
        ("vstack-2child", row_vstack_2child),
        ("inexact-sum", row_inexact_sum),
        ("long-wrap", row_long_wrap),
    ];

    let mut mismatches = Vec::new();
    for (name, make_row) in shapes {
        let proposal = ProposalSize::new(Some(308.0), None);
        let lazy = AnyView::new(VStack::for_each(
            (0..40).map(SelfId::new).collect::<Vec<_>>(),
            move |_| make_row(),
        ));
        let lazy_answer = Realized::with_theme(lazy, &env, &theme)
            .measure(proposal)
            .size;
        let row_view = make_row();
        let item_answer = Realized::with_theme(row_view, &env, &theme)
            .measure(proposal)
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

/// The issue's shell: a `scroll` wrapping the
/// lazy stack must report the offered cross extent — the content's intrinsic
/// is what the viewport clips, not what the scroll is.
#[test]
#[expect(
    clippy::float_cmp,
    reason = "the scroll shell must echo its offered width unchanged, not recompute it"
)]
fn scroll_shell_answers_its_proposal() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let shell = AnyView::new(scroll(vstack((VStack::for_each(
        (0..40).map(SelfId::new).collect::<Vec<_>>(),
        move |_| row_long_wrap(),
    ),))));
    let answer = Realized::with_theme(shell, &env, &theme)
        .measure(ProposalSize::new(Some(308.0), None))
        .size;
    assert!(
        answer.width == 308.0,
        "scroll wrapping a lazy stack answered {answer:?} under (308, None): left {:?}, right {:?}",
        answer.width,
        308.0
    );
}

#[test]
fn for_each_virtualizes_only_inside_coaxial_scroll_unless_eager() {
    use waterui::layout::scroll::{scroll_both, scroll_horizontal};

    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());

    let coaxial_v = Realized::with_theme(
        scroll(VStack::for_each(["a", "b", "c"], text)),
        &env,
        &theme,
    );
    assert!(
        coaxial_v.node.has_lazy_stack(),
        "scroll(VStack::for_each(..)) must virtualize along the vertical scroll axis"
    );

    let eager_opt_out = Realized::with_theme(
        scroll(VStack::for_each(["a", "b", "c"], text).eager()),
        &env,
        &theme,
    );
    assert!(
        eager_opt_out.node.has_collection() && !eager_opt_out.node.has_lazy_stack(),
        "VStack::for_each(..).eager() must stay eager inside a vertical scroll"
    );

    let bare_v = Realized::with_theme(VStack::for_each(["a", "b", "c"], text), &env, &theme);
    assert!(
        bare_v.node.has_collection() && !bare_v.node.has_lazy_stack(),
        "bare VStack::for_each(..) outside a ScrollView must materialize eagerly"
    );

    let orthogonal = Realized::with_theme(
        scroll_horizontal(VStack::for_each(["a", "b", "c"], text)),
        &env,
        &theme,
    );
    assert!(
        orthogonal.node.has_collection() && !orthogonal.node.has_lazy_stack(),
        "VStack::for_each(..) inside a horizontal-only ScrollView must stay eager"
    );

    let both_h = Realized::with_theme(
        scroll_both(HStack::for_each(["a", "b", "c"], text)),
        &env,
        &theme,
    );
    assert!(
        both_h.node.has_lazy_stack(),
        "scroll_both(HStack::for_each(..)) must virtualize along the horizontal axis"
    );
}

#[test]
fn lazy_stack_propagates_sampled_cross_stretch_to_enclosing_scroll() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());

    let stretching = Realized::with_theme(
        scroll(VStack::for_each(["a", "b", "c"], |label| {
            hstack((text(label), spacer()))
        })),
        &env,
        &theme,
    );
    assert_eq!(
        stretching.stretch_axis(),
        StretchAxis::Both,
        "a vertical scroll over horizontally-stretching lazy rows must stretch on both axes"
    );

    let intrinsic = Realized::with_theme(
        scroll(VStack::for_each(["a", "b", "c"], text)),
        &env,
        &theme,
    );
    assert_eq!(
        intrinsic.stretch_axis(),
        StretchAxis::Vertical,
        "a vertical scroll over content-sized lazy rows stretches only vertically"
    );
}
