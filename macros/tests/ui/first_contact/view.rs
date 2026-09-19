//! `vstack((ForEach::new(rows, f),))`: `ForEach` is a collection of views,
//! not a view, so the stack rejects it on `View`.

use waterui::Identifiable;
use waterui::prelude::*;
use waterui::views::ForEach;

#[derive(Clone, Identifiable)]
struct Row {
    #[id]
    id: u64,
    name: &'static str,
}

fn broken() -> impl View {
    let rows = vec![Row { id: 1, name: "one" }];
    vstack((ForEach::new(rows, |row| text(row.name)),))
}

fn main() {}
