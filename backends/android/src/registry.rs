//! The registration table: every claim the backend makes, in one place.
//!
//! A component port adds a `components/<name>.rs` module exposing
//! `pub fn install(&mut Dispatcher)`, a `#[cfg(feature = "<name>")]`
//! line here, and the matching Cargo feature. This file is the shared merge
//! point the coordinator integrates.

use crate::dispatch::Dispatcher;

/// Fills `dispatcher` with every claim the backend owns. Called exactly
/// once, before the first render, inside [`crate::dispatch::dispatcher`].
pub fn install(dispatcher: &mut Dispatcher) {
    // Core — the unit view, always claimed: an unclaimed `Native<()>` falls
    // through the walk into a `body()` panic, which `panic = "abort"` makes fatal.
    crate::components::empty::install(dispatcher);

    // Environment overlay — `Metadata<Environment>` is core env machinery
    // (every `.env(...)` wraps content in it), not a feature-gated port:
    // `body()` panics whenever no handler claims it.
    crate::components::with_env::install(dispatcher);

    // Reactive content — `Dynamic::watch`/`text!`-bound content arrives as
    // `Native<Dynamic>`; unclaimed it falls into the same `body()` panic.
    crate::components::dynamic::install(dispatcher);

    // The skeleton's wave: text, the stack container, and button.
    #[cfg(feature = "text")]
    crate::components::text::install(dispatcher);
    #[cfg(feature = "button")]
    crate::components::button::install(dispatcher);
    #[cfg(feature = "container")]
    crate::components::container::install(dispatcher);
}
