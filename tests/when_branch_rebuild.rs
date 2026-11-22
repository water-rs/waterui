//! `when(..)` must not rebuild the active branch while the selected branch
//! stays the same.

use std::cell::Cell;
use std::rc::Rc;

use waterui::Binding;
use waterui::SignalExt as _;
use waterui::component::text;
use waterui::widget::condition::when;
use waterui_testing::ui;

#[test]
fn when_does_not_rebuild_unchanged_branch() {
    let items: Binding<Vec<u32>> = Binding::container(vec![1]);
    let builds = Rc::new(Cell::new(0_u32));
    let counter = Rc::clone(&builds);
    let flag = items.clone();
    let mut app = ui().viewport(400, 300).mount(move || {
        let counter = Rc::clone(&counter);
        when(flag.clone().map(|v| !v.is_empty()), move || {
            counter.set(counter.get() + 1);
            text("list")
        })
        .otherwise(|| text("empty"))
    });
    app.settle();
    let before = builds.get();
    assert!(before >= 1);

    // The condition stays `true`: the branch must not be rebuilt.
    items.set(vec![1, 2]);
    app.settle();
    items.set(vec![1, 2, 3]);
    app.settle();
    assert_eq!(
        builds.get(),
        before,
        "active branch rebuilt without a branch change"
    );
}
