//! `#[state]` takes no arguments; any token inside the parentheses is an
//! error.

use waterui::prelude::*;

#[state(channel)]
struct ArgsRejected;

fn main() {}
