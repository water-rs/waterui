//! Headless rendering and accessibility-first test utilities for `WaterUI`.
//!
//! `waterui-testing` runs inside ordinary `cargo test` targets. [`ui`] builds a
//! test session; theme and render mode are orthogonal: [`UiBuilder::theme`]
//! swaps the theme package (the plain Hydrolysis test theme by default) and
//! [`UiBuilder::mount`] / [`UiBuilder::mount_offscreen`] pick between the fast
//! semantic runtime and the GPU-backed offscreen runtime.
//!
//! # `cargo test` Integration
//!
//! ```ignore
//! fn login_view() -> impl waterui::View {
//!     waterui::text("Login").body()
//! }
//!
//! #[waterui::test(login_view, theme = hydrolysis_m3::install)]
//! fn login_smoke(app: &mut waterui_testing::SemanticApp) {
//!     app.query()
//!         .role(waterui_testing::Role::LABEL)
//!         .label("Login")
//!         .assert_exists();
//! }
//! ```
//!
//! For tests that own `Binding`s the view closes over, omit the view path and
//! take the configured [`UiBuilder`] by value (the manual-mount form):
//!
//! ```ignore
//! #[waterui::test(theme = hydrolysis_m3::install)]
//! fn stepper_updates(ui: waterui_testing::UiBuilder) {
//!     let value = waterui::Binding::i32(2);
//!     let value_for_view = value.clone();
//!     let mut app = ui.mount(move || stepper("Limited", &value_for_view));
//!     app.query().label("Limited").increment();
//!     assert_eq!(value.get(), 3);
//! }
//! ```
//!
//! The `#[waterui::test(...)]` macro expands to a regular `#[test]`, so these
//! tests run under the normal Rust test harness and on GitHub Actions without a
//! custom runner.
//!
//! # Interactions, waits, and time
//!
//! Interactions (`tap`, `set_text`, `increment`, ...) return `()` and panic
//! when the runtime reports the accessibility action unhandled — a plain call
//! is the assertion. After every interaction the session settles to real
//! quiescence (no queued input, no spawned work, no scheduled animations or
//! patches) instead of sleeping. The animation clock is virtual: each pump
//! advances it exactly one frame, so transition sampling is deterministic;
//! [`OffscreenApp::pump_for`] lands on an exact phase of a transition, and
//! waits (`wait_for_existence`, [`SemanticApp::wait_for`]) pump hot while work
//! is scheduled and only touch wall-clock time for work outside the runtime.
//!
//! Use semantic queries to resolve an [`ElementRef`], then drive interactions
//! through that handle or use it to scope later queries with [`Query::within`].
//! Views tagged with `.a11y_id("login.submit")` resolve via
//! `Selector::identifier` / `Query::identifier`.
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
mod executor;
mod perf;
pub mod protocol;
mod query;
mod selector;
mod semantics;
mod snapshot;
pub(crate) mod wait;

pub use accesskit::Role as AccessKitRole;
pub use app::{DragOptions, OffscreenApp, SemanticApp, ThemeInstaller, UiBuilder, ui};
pub use executor::drain_parked_local_work;
pub use hydrolysis::{KeyCode, Modifiers};
pub use artifacts::{CapturedSnapshot, TestArtifacts, artifact_root};
pub use driver::FrameTiming;
pub use executor::{TestLocalExecutor, install_test_executor};
pub use perf::{PerfApp, PerfConfig, PerfMeasurement, PerfReport, PerfRun, PerfStats};
pub use query::Query;
pub use selector::{ElementRef, ElementSet, Selector};
pub use semantics::{CheckedState, NodeBounds, NodeId, NodeSnapshot, Role, TreeSnapshot};
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
