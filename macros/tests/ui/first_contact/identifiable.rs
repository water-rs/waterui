//! `Lazy::for_each(rows, f)`: `ForEach` diffs by id, so the row type must
//! implement `Identifiable` — `#[derive(Identifiable)]` with `#[id]` on the
//! identifier field is the intended fix.

use waterui::component::lazy::Lazy;
use waterui::prelude::*;

#[derive(Clone)]
struct Row {
    id: u64,
    name: &'static str,
}

fn broken() -> impl View {
    let rows = vec![Row { id: 1, name: "one" }];
    Lazy::for_each(rows, |row| text(row.name))
}

fn main() {}
