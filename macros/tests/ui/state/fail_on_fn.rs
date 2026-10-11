//! `#[state]` applies to a struct or enum; anything else is rejected with a
//! named diagnostic on the item.

use waterui::prelude::*;

#[state]
fn not_a_state_type() {}

fn main() {}
