//! `.selection` of `usize` on a `List::content` whose rows are tagged by
//! `&str`: a row is selected only by the type it is tagged with.

use waterui::Binding;
use waterui::component::list::{List, row};
use waterui::prelude::*;

fn broken(selection: &Binding<Option<usize>>) -> impl View + use<> {
    List::content((
        row("Storage", "3 GB"),
        row("Inbox", "12").tag("inbox"),
        row("下書き", "2").tag("drafts"),
    ))
    .selection(selection)
}

fn main() {}
