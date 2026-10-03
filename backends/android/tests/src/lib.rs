//! The `waterui-android` test application: a text view, a stack container,
//! and a button whose press flips a `Binding` a reactive label reads — the
//! in-place update proves signals reach the platform without a subtree
//! rebuild. A `VStack::for_each` over a `nami::collection::List`, plus two
//! buttons that push and pop rows, exercises the lazy container's
//! incremental membership: rows appear and disappear on the collection's
//! emissions, not on a subtree rebuild.

use waterui::Environment;
use waterui::app::App;
use waterui::id::SelfId;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::reactive::collection::List as ReactiveList;

/// `export_app!`'s entry: declare the app the host mounts.
pub fn app(env: Environment) -> App {
    App::new(main, env)
}

fn main() -> impl View {
    let taps = binding::<i32>(0);
    let rows = ReactiveList::from(vec![SelfId::new(1), SelfId::new(2)]);
    vstack((
        text!("WaterUI on Android"),
        button("Tap me").action({
            let taps = taps.clone();
            move || *taps.get_mut() += 1
        }),
        text!("Pressed {taps} times"),
        VStack::for_each(rows.clone(), |row| text(format!("Row {}", *row))),
        button("Add row").action({
            let rows = rows.clone();
            let mut next = 3;
            move || {
                rows.push(SelfId::new(next));
                next += 1;
            }
        }),
        button("Remove last row").action(move || {
            let _ = rows.pop();
        }),
    ))
    .padding()
}

waterui_android::export_app!(app);
