//! `List::content((text("Inbox"), row("Archive", "12")))`: a finished view is
//! not `ListContent`; static rows must be row builders (`|| text("...")`) or
//! `row(...)` / `detail_row(...)` values.

use waterui::component::list::{List, row};
use waterui::prelude::*;

fn broken() -> impl View {
    List::content((text("Inbox"), row("Archive", "12")))
}

fn main() {}
