//! Headless rendering and accessibility-first test utilities for `WaterUI`.
//!
//! `waterui-testing` is designed to run inside ordinary `cargo test` targets.
//! Use [`ui`] for interactive accessibility-first flows. Passing a theme installer
//! enables offscreen Hydrolysis rendering and performance measurement.
//!
//! # `cargo test` Integration
//!
//! ```ignore
//! fn login_view() -> impl waterui::View {
//!     waterui::text("Login").body()
//! }
//!
//! #[waterui::test(login_view)]
//! fn login_smoke(app: &mut waterui_testing::SemanticApp) {
//!     let login = app
//!         .query()
//!         .role(waterui_testing::Role::LABEL)
//!         .label("Login")
//!         .single();
//!     assert_eq!(login.node().label(), Some("Login"));
//! }
//! ```
//!
//! The `#[waterui::test(...)]` macro expands to a regular `#[test]`, so these
//! tests run under the normal Rust test harness and on GitHub Actions without a
//! custom runner.
//! Use semantic queries to resolve an [`ElementRef`], then drive interactions
//! through that handle or use it to scope later queries with [`Query::within`].
//!
//! # Snapshot Artifacts
//!
//! ```ignore
//! use waterui::Environment;
//! use waterui::ViewExt as _;
//! use waterui::graphics::color::Srgb;
//! use waterui_testing::TestHost;
//!
//! let host = TestHost::new(Environment::new(), 320, 180);
//! let captured = host.capture_snapshot(
//!     waterui::text("Preview")
//!         .body()
//!         .foreground(Srgb::WHITE)
//!         .background(Srgb::BLACK),
//!     "docs/visual",
//!     "text-preview",
//!     "00_initial",
//! );
//! assert!(captured.path().is_file());
//! ```
//!
//! When `WATERUI_TEST_ARTIFACTS_DIR` is set, snapshots are written beneath that
//! directory using `WaterUI`'s canonical `<suite>/<case>/<stage>.png` layout. The
//! repository's GitHub workflows already upload and summarize those snapshot images.

mod app;
mod artifacts;
pub(crate) mod driver;
mod perf;
mod query;
mod selector;
mod semantics;
mod snapshot;
pub(crate) mod wait;

pub use app::{OffscreenApp, SemanticApp, ThemeInstaller, UiBuilder, ui};
pub use artifacts::{CapturedSnapshot, TestArtifacts, artifact_root};
pub use driver::FrameTiming;
pub use perf::{PerfApp, PerfConfig, PerfMeasurement, PerfReport, PerfRun, PerfStats};
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
