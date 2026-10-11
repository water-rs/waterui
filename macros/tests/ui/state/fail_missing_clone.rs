//! `#[state]` requires `Clone + 'static`: the assertion the macro emits inside
//! the generated `extract` reports the missing bound against the attribute.

use waterui::prelude::*;

#[state]
struct MissingClone {
    doc: Str,
}

fn main() {}
