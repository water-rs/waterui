//! CLI command implementations.

use std::path::{Path, PathBuf};

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
pub mod channel;
pub mod clean;
pub mod create;
pub mod device;
pub mod devices;
pub mod doctor;
pub mod gc;
pub mod inspector;
pub mod mcp;
pub mod package;
pub mod preview;
pub mod run;

/// Parse frame size from a `WIDTHxHEIGHT` string.
fn parse_frame(s: &str) -> color_eyre::eyre::Result<(f32, f32)> {
    let parts: Vec<&str> = s.split('x').collect();
    if parts.len() != 2 {
        color_eyre::eyre::bail!("Invalid frame format: expected WIDTHxHEIGHT (e.g., 375x667)");
    }

    let width: f32 = parts[0]
        .parse()
        .map_err(|_| color_eyre::eyre::eyre!("Invalid frame width"))?;
    let height: f32 = parts[1]
        .parse()
        .map_err(|_| color_eyre::eyre::eyre!("Invalid frame height"))?;

    if !width.is_finite() || width <= 0.0 {
        color_eyre::eyre::bail!("Invalid frame width: must be a positive finite number");
    }
    if !height.is_finite() || height <= 0.0 {
        color_eyre::eyre::bail!("Invalid frame height: must be a positive finite number");
    }

    Ok((width, height))
}

/// Parse a viewport size from a `WIDTHxHEIGHT` string into whole pixels.
fn parse_viewport(s: &str) -> color_eyre::eyre::Result<(u32, u32)> {
    let parts: Vec<&str> = s.split('x').collect();
    if parts.len() != 2 {
        color_eyre::eyre::bail!("Invalid viewport format: expected WIDTHxHEIGHT (e.g., 390x844)");
    }

    let width: u32 = parts[0]
        .parse()
        .map_err(|_| color_eyre::eyre::eyre!("Invalid viewport width"))?;
    let height: u32 = parts[1]
        .parse()
        .map_err(|_| color_eyre::eyre::eyre!("Invalid viewport height"))?;

    if width == 0 {
        color_eyre::eyre::bail!("Invalid viewport width: must be positive");
    }
    if height == 0 {
        color_eyre::eyre::bail!("Invalid viewport height: must be positive");
    }

    Ok((width, height))
}

/// Reads the `package.name` of a project's `Cargo.toml` — the crate name the
/// generated backend depends on.
async fn read_project_crate_name(project_path: &Path) -> color_eyre::eyre::Result<String> {
    let cargo_toml = project_path.join("Cargo.toml");
    let cargo_content = smol::fs::read_to_string(&cargo_toml).await?;
    let cargo: toml::Table = cargo_content.parse()?;
    cargo
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| color_eyre::eyre::eyre!("Could not find package name in Cargo.toml"))
}
