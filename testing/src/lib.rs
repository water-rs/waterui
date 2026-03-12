//! Headless rendering and accessibility-first test utilities for WaterUI.

mod app;
mod artifacts;
pub(crate) mod driver;
mod query;
mod selector;
mod semantics;
mod snapshot;
mod wait;

pub use app::{MountedApp, UiTest};
pub use artifacts::{TestArtifacts, artifact_root};
pub use query::Query;
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
