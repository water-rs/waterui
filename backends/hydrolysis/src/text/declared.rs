//! Font files a host resolved from font declarations.

use std::path::PathBuf;

/// The font files a host resolved from font declarations, registered by the
/// runtime's font collection in addition to the
/// [`waterui_core::ResourceContext`] fonts directory.
///
/// An application never carries one: the `water` CLI stages the fonts its
/// dependency graph declares into the fonts directory. A host that runs
/// without that staging step — a test binary under `cargo test` — resolves
/// the declarations itself and installs the files it found here.
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
