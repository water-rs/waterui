//! `.selection` on a `List::content` whose rows are all untagged: no row can
//! be selected, so no selection type fits the list.

use waterui::Binding;
use waterui::component::list::{List, ListItem, Section, row};
use waterui::component::text;
use waterui::prelude::*;

fn broken(selection: &Binding<Option<usize>>) -> impl View + use<> {
    List::content((
        row("Storage", "3 GB"),
        Section::new("Mailboxes").content((
            || ListItem::new(text("Inbox")),
            row("下書き", "2"),
            row("받은 편지함", "5"),
        )),
    ))
    .selection(selection)
}

fn main() {}
