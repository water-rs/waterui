//! Minimal release application fixture for the reachability gate (#4694):
//! only `text` and `button`.

use waterui::app::{App, Windows};
use waterui::prelude::*;

fn main_view() -> impl View {
    let count = Binding::i32(0);
    vstack((
        text!("Count: {count}"),
        button("Increment")
            .action(|State(count): State<Binding<i32>>| {
                *count.get_mut() += 1;
            })
            .state(&count),
    ))
}

/// Builds the minimal reachability fixture application.
pub fn app(env: Environment) -> App<impl Windows> {
    App::new(main_view, env)
}
