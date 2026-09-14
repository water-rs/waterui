//! `text(&name)`: a borrowed `&String` is not `IntoText`; text positions take
//! `&'static str`, `String`, `Str`, `StyledStr`, `Text`, or a `Binding` /
//! `Computed` signal.

use waterui::prelude::*;

fn broken(name: &String) -> impl View {
    text(name)
}

fn main() {}
