//! `LazyContainer::new(VStackLayout::default(), rows)`: a reactive data
//! collection is not a `Views` collection — its items are data rows, not views.
//! `VStack::for_each` maps each row to a view instead.

use waterui::Identifiable;
use waterui::layout::stack::VStackLayout;
use waterui::prelude::*;

#[derive(Clone, Identifiable)]
struct Row {
    #[id]
    id: u64,
    name: &'static str,
}

fn broken() -> impl View {
    let rows = StateList::from(vec![Row { id: 1, name: "one" }]);
    LazyContainer::new(VStackLayout::default(), rows)
}

fn main() {}
