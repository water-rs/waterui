//! `Lazy::vstack(rows)`: a reactive data collection is not a `Views`
//! collection — its items are data rows, not views. `Lazy::for_each` maps
//! each row to a view instead.

use waterui::Identifiable;
use waterui::component::lazy::Lazy;
use waterui::prelude::*;
use waterui::reactive::collection::List as ReactiveList;

#[derive(Clone, Identifiable)]
struct Row {
    #[id]
    id: u64,
    name: &'static str,
}

fn broken() -> impl View {
    let rows = ReactiveList::from(vec![Row { id: 1, name: "one" }]);
    Lazy::vstack(rows)
}

fn main() {}
