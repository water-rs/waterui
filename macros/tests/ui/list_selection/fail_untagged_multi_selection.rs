//! `.multi_selection` on a `List::content` whose rows are all untagged: no
//! row can be selected, so no selection type fits the list.

use std::collections::BTreeSet;

use waterui::Binding;
use waterui::component::list::{List, row};
use waterui::component::text;
use waterui::prelude::*;

fn broken(selection: &Binding<BTreeSet<&'static str>>) -> impl View + use<> {
    List::content((
        || text("Buy milk"),
        row("牛乳を買う", "today"),
        row("여권 갱신", "next week"),
    ))
    .multi_selection(selection)
}

fn main() {}
