//! `#[state]` applies to a struct or enum; a union is rejected on the `union`
//! keyword — its fields cannot satisfy `Clone` through a derive.

use waterui::prelude::*;

#[state]
union NotState {
    word: u32,
}

fn main() {}
