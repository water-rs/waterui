//! Accessibility-first test utilities for `WaterUI`, on Hydrolysis's two
//! headless pipelines.
//!
//! `waterui-testing` runs inside ordinary `cargo test` targets. [`ui`] builds
//! a test session on the *semantic* runtime — a GPU-free pipeline whose
//! accessibility tree is a product of the view tree and the widgets'
//! semantics, so it carries no style package and answers no geometry:
//! [`UiBuilder::mount`] is all a `UiBuilder<NoStyle>` offers. Tests that need
//! pixels, bounds, pointer gestures or frame timing carry a
//! [`hydrolysis::Style`] through [`UiBuilder::theme`] and mount the *rendered*
//! runtime through `mount_offscreen` — the distinction is in the type, not a
//! runtime check, so a semantic query has no `bounds()` method.
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
//! #[waterui::test]
//! fn stepper_updates(ui: waterui_testing::UiBuilder) {
//!     let value = waterui::Binding::i32(2);
//!     let value_for_view = value.clone();
//!     let mut app = ui.mount(move || stepper("Limited", &value_for_view));
//!     app.query().label("Limited").increment();
//!     assert_eq!(value.snapshot(), 3);
//! }
//! ```
//!
//! A rendered test names its style with `theme =`, and a whole [`App`]
//! mounts through [`mount_app`] — or `ui().theme(style).mount_app(app)` when
//! the session needs its own viewport, runtime flavor or scale factor:
//!
//! ```ignore
//! #[waterui::test(login_view, theme = hydrolysis_m3::Material3::defaults(), offscreen)]
//! fn login_rendered(app: &mut waterui_testing::OffscreenApp) {
//!     let snapshot = app.snapshot();
//!     assert_eq!(snapshot.width, 390);
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
//! waits (`wait_for_existence`, `SemanticApp::wait_for`) pump hot while work
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
//! let host = TestHost::new(
//!     Environment::new(),
//!     320,
//!     180,
//!     hydrolysis_m3::Material3::defaults(),
//! );
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
pub mod bench;
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
pub use app::{
    DragOptions, NoStyle, OffscreenApp, RuntimeFlavor, SemanticApp, Styled, UiBuilder, mount_app,
    ui,
};
pub use artifacts::{CapturedSnapshot, TestArtifacts, artifact_root};
pub use driver::{FrameTiming, RuntimeDriver, VIRTUAL_FRAME};
pub use executor::drain_parked_local_work;
pub use executor::{TestLocalExecutor, install_test_executor};
pub use hydrolysis::{HeadlessRuntime, KeyCode, Modifiers, SemanticRuntime, Style};
pub use perf::{PerfApp, PerfConfig, PerfMeasurement, PerfReport, PerfRun, PerfStats};
pub use query::Query;
pub use selector::{ElementAnchor, ElementRef, ElementSet, Selector};
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
