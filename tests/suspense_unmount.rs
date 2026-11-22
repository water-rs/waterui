//! A `suspense(..)` loading future must be cancelled when the view is
//! unmounted while it is still loading.

use std::cell::Cell;
use std::future::pending;
use std::rc::Rc;

use waterui::Binding;
use waterui::SignalExt as _;
use waterui::component::text;
use waterui::widget::condition::when;
use waterui::widget::suspense::suspense;
use waterui_testing::ui;

struct DropFlag(Rc<Cell<bool>>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn suspense_future_dropped_on_unmount() {
    let shown: Binding<bool> = Binding::container(true);
    let dropped = Rc::new(Cell::new(false));
    let flag = shown.clone();
    let probe = Rc::clone(&dropped);
    let mut app = ui().viewport(400, 300).mount(move || {
        let probe = Rc::clone(&probe);
        when(flag.clone().map(|v| v), move || {
            let guard = DropFlag(Rc::clone(&probe));
            suspense(async move {
                let _guard = guard;
                pending::<()>().await;
                text("never")
            })
        })
        .otherwise(|| text("gone"))
    });
    app.settle();
    assert!(!dropped.get(), "future must still be loading while mounted");

    shown.set(false);
    app.settle();
    app.settle();
    assert!(
        dropped.get(),
        "loading future still alive after the Suspense was unmounted"
    );
}
