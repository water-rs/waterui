//! `.on_change(..)` resolves its handler in the environment its modified view
//! resolves in, so a `.state(..)` install reaches the handler on either side of
//! it in the modifier chain, as `ViewExt::state` documents.

use waterui::Binding;
use waterui::Signal as _;
use waterui::State;
use waterui::ViewExt as _;
use waterui::component::text;
use waterui_testing::ui;

#[test]
fn on_change_sees_state_installed_before_it() {
    let source: Binding<i32> = Binding::i32(0);
    let total: Binding<i32> = Binding::i32(0);
    let (src, tot) = (source.clone(), total.clone());
    let mut app = ui().viewport(400, 300).mount(move || {
        text("x")
            .state(&tot)
            .on_change(&src, |v: i32, State(t): State<Binding<i32>>| t.set(v))
    });
    app.settle();
    source.set(7);
    app.settle();
    assert_eq!(
        total.snapshot(),
        7,
        "handler did not see .state() placed before .on_change()"
    );
}

#[test]
fn on_change_sees_state_installed_after_it() {
    let source: Binding<i32> = Binding::i32(0);
    let total: Binding<i32> = Binding::i32(0);
    let (src, tot) = (source.clone(), total.clone());
    let mut app = ui().viewport(400, 300).mount(move || {
        text("x")
            .on_change(&src, |v: i32, State(t): State<Binding<i32>>| t.set(v))
            .state(&tot)
    });
    app.settle();
    source.set(7);
    app.settle();
    assert_eq!(
        total.snapshot(),
        7,
        "handler did not see .state() placed after .on_change()"
    );
}

#[test]
fn on_change_sees_state_installed_before_another_handler() {
    let source: Binding<i32> = Binding::i32(0);
    let total: Binding<i32> = Binding::i32(0);
    let (src, tot) = (source.clone(), total.clone());
    let mut app = ui().viewport(400, 300).mount(move || {
        text("x")
            .state(&tot)
            .on_hover_enter(|| {})
            .on_change(&src, |v: i32, State(t): State<Binding<i32>>| t.set(v))
    });
    app.settle();
    source.set(7);
    app.settle();
    assert_eq!(
        total.snapshot(),
        7,
        "handler did not see .state() placed before an inner handler in its chain"
    );
}
