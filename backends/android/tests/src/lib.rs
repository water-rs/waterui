//! The `waterui-android` test application: a text view, a stack container,
//! and a button whose press flips a `Binding` a reactive label reads — the
//! in-place update proves signals reach the platform without a subtree
//! rebuild.

use waterui::Environment;
use waterui::app::App;
use waterui::prelude::*;
use waterui::reactive::binding;

/// `export_app!`'s entry: declare the app the host mounts.
pub fn app(env: Environment) -> App {
    App::new(main, env)
}

fn main() -> impl View {
    let taps = binding::<i32>(0);
    vstack((
        text!("WaterUI on Android"),
        button("Tap me").action({
            let taps = taps.clone();
            move || *taps.get_mut() += 1
        }),
        text!("Pressed {taps} times"),
    ))
    .padding()
}

waterui_android::export_app!(app);
