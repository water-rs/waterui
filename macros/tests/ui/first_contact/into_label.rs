//! `button(maybe_title)`: `Option<&'static str>` is not `IntoLabel`; a
//! control label takes `&'static str`, `String`, `Str`, `StyledStr`, `Text`,
//! `Label`, or a `Binding` / `Computed` signal.

use waterui::prelude::*;

fn broken(maybe_title: Option<&'static str>) -> impl View {
    button(maybe_title)
}

fn main() {}
