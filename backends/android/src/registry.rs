//! The registration table: every claim the backend makes, in one place.
//!
//! A component port adds a `components/<name>.rs` module exposing
//! `pub(crate) fn install(&mut Dispatcher)`, a `#[cfg(feature = "<name>")]`
//! line here, and the matching Cargo feature. This file is the shared merge
//! point the coordinator integrates.

use crate::dispatch::Dispatcher;

/// Fills `dispatcher` with every claim the backend owns. Called exactly
/// once, before the first render, inside [`crate::dispatch::dispatcher`].
pub fn install(dispatcher: &mut Dispatcher) {
    // Core — the unit view, always claimed: an unclaimed `Native<()>` falls
    // through the walk into a `body()` panic, which `panic = "abort"` makes fatal.
    crate::components::empty::install(dispatcher);

    // The skeleton's wave: text, the stack container, and button.
    #[cfg(feature = "text")]
    crate::components::text::install(dispatcher);
    #[cfg(feature = "button")]
    crate::components::button::install(dispatcher);
    #[cfg(feature = "container")]
    crate::components::container::install(dispatcher);
}
