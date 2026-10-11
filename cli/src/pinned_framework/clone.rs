//! The framework checkout the tests resolve — the part of the module the
//! `water` binary's tests compile too (`src/terminal/main.rs` includes this
//! file by path), so it must not name the GitHub helpers beside it.

use std::{path::Path, path::PathBuf};

/// The enclosing workspace checkout.
///
/// Dereferences to the repository root the crate is built inside — the
/// directory `water create --waterui-path` would name for this revision.
/// Callers hold it by value so the path exists for as long as the test
/// works inside the tree; unlike the standalone repository's clone, the
/// tree is checked out, not materialized per test.
pub struct PinnedCheckout {
    directory: PathBuf,
}

impl std::ops::Deref for PinnedCheckout {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.directory
    }
}

/// The workspace checkout this crate lives in.
///
/// The standalone repository cloned the pinned framework revision into
/// `target/pinned-framework/` behind a shared lock; in-tree the "pin" is the
/// enclosing checkout itself — the same revision `cargo` resolves the
/// `waterui-*` path dependencies from — so the function is a path
/// resolution, not a clone. Tests that scaffold or `cargo metadata` against
/// real framework manifests read this tree directly.
pub fn checkout() -> PinnedCheckout {
    // `CARGO_MANIFEST_DIR` is `<workspace>/cli`; the framework checkout is
    // its parent. A packaged build's parent is the tarball, which has no
    // workspace root — the assert below fails there with a named cause.
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let directory = manifest_dir
        .parent()
        .unwrap_or_else(|| {
            panic!(
                "{} has no parent directory to serve as the framework checkout",
                manifest_dir.display()
            )
        })
        .to_path_buf();
    assert!(
        directory.join("Cargo.toml").is_file(),
        "the framework checkout is the workspace root containing this crate; \
         {} carries no root manifest — run the test inside the water-rs/waterui \
         checkout the crate lives in",
        directory.display()
    );
    PinnedCheckout { directory }
}
