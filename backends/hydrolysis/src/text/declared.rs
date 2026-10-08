//! Font files a host resolved from font declarations.

use std::path::PathBuf;

/// The font files a host resolved from font declarations, registered by the
/// runtime's font collection in addition to the
/// [`waterui_core::ResourceContext`] fonts directory.
///
/// A host that staged the graph's declared fonts into the fonts directory
/// installs an empty one — the `water` CLI's runtime binaries do, since the
/// CLI already staged them — and a host that installs none at all — a test
/// binary under `cargo test` — has the harness resolve the package under
/// test's declarations itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeclaredFonts {
    paths: Vec<PathBuf>,
}

impl DeclaredFonts {
    /// The declared font files at `paths`, each a `.ttf`/`.otf` the runtime
    /// registers as it registers a staged one.
    #[must_use]
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            paths: paths.into_iter().collect(),
        }
    }

    /// The declared font files.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }
}
