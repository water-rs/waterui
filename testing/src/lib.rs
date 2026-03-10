//! Headless rendering and accessibility-first test utilities for WaterUI.

mod app;
pub(crate) mod driver;
mod selector;
mod semantics;
mod snapshot;
mod wait;

pub use app::{MountedApp, Query, UiTest};
pub use selector::{ElementRef, ElementSet, Selector};
pub use semantics::{NodeBounds, NodeId, NodeSnapshot, Role, TreeSnapshot};
pub use snapshot::{Snapshot, TestHost};
pub use wait::{Expectation, WaitOptions, WaitResult};

/// Internal async bridge used by `#[waterui::test(...)]` expansion.
pub fn block_on<F>(future: F) -> F::Output
where
    F: core::future::Future,
{
    pollster::block_on(future)
}

#[cfg(test)]
mod tests;
