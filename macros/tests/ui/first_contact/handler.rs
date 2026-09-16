//! `.action(|count: Binding<i32>| ..)`: a handler parameter must implement
//! `Extractor`; a bare `Binding<i32>` does not — `State<Binding<i32>>` paired
//! with `.state(&count)` does.

use waterui::prelude::*;

fn broken() -> impl View {
    button("Increment").action(|count: Binding<i32>| *count.get_mut() += 1)
}

fn main() {}
