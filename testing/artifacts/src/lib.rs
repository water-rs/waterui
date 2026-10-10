//! The canonical artifact store `WaterUI` test suites write into.
//!
//! A run's evidence — snapshot PNGs and any companion captures — lands under
//! [`artifact_root`] in a `<suite>/<case>/<stage>.png` layout owned by
//! [`TestArtifacts`]. The root is `WATERUI_TEST_ARTIFACTS_DIR` when a
//! workflow sets it (the repository's workflows upload and summarize that
//! directory) and the platform temp directory otherwise.
//!
//! The store is deliberately free of any renderer or harness dependency:
//! suites compiled into a backend's own `cargo test --lib` target name it
//! directly, where depending on a harness crate like `waterui-testing`
//! would drag that backend's rlib into the link a second time
//! (water-rs/waterui#2447).

mod artifacts;
mod snapshot;

pub use artifacts::{CapturedSnapshot, TestArtifacts, artifact_root};
pub use snapshot::Snapshot;
