//! CLI command implementations.

use std::path::PathBuf;

use crate::shell::Shell;
use crate::{note, warn};
use waterui_cli::{toolchain::sccache::Sccache, utils::sccache_install_hint};

/// Whether the environment permits routing builds through `sccache`.
fn sccache_allowed() -> bool {
    if let Some(value) = std::env::var_os("WATERUI_DISABLE_SCCACHE") {
        let value = value.to_string_lossy().trim().to_ascii_lowercase();
        if matches!(value.as_str(), "1" | "true" | "yes" | "on") {
            return false;
        }
    }
    // Respect explicit wrapper from caller (e.g. passthrough wrapper in constrained envs).
    std::env::var_os("RUSTC_WRAPPER").is_none()
}

/// Locate `sccache` for compilation caching, noting on the shell when it is
/// skipped or missing.
async fn detect_sccache_path(shell: &Shell) -> Option<PathBuf> {
    if !sccache_allowed() {
        note!(
            shell,
            "Skipping sccache (explicit wrapper or WATERUI_DISABLE_SCCACHE is set)"
        );
        return None;
    }

    let sccache = Sccache;
    sccache.path().await.map_or_else(
        |_| {
            warn!(
                shell,
                "sccache not found. Build efficiency may be reduced. Install with: {}",
                sccache_install_hint()
            );
            None
        },
        Some,
    )
}

pub mod backend;
pub mod bench;
pub mod build;
pub mod clean;
pub mod create;
pub mod device;
pub mod devices;
pub mod doctor;
pub mod gc;
pub mod inspector;
pub mod package;
pub mod preview;
pub mod run;
