use std::{env, path::Path, process::Command};

use color_eyre::eyre::Result;
use tracing::warn;
use which::which;

/// Ensure a directory exists, creating missing parents when needed.
///
/// # Errors
/// Returns an error if the directory cannot be created.
pub fn ensure_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Inject standard environment variables that toggle `WaterUI` hot reload at runtime.
pub fn configure_hot_reload_env(cmd: &mut Command, enable: bool, port: Option<u16>) {
    if enable {
        cmd.env("WATERUI_DISABLE_HOT_RELOAD", "0");
        if let Some(port) = port {
            cmd.env("WATERUI_HOT_RELOAD_PORT", port.to_string());
        }
    } else {
        cmd.env("WATERUI_DISABLE_HOT_RELOAD", "1");
    }
}

/// Apply build speedups (sccache, mold) to a Cargo command.
///
/// Returns `true` if sccache was enabled so callers can retry without it when builds fail.
pub fn configure_build_speedups(
    cmd: &mut Command,
    enable_sccache: bool,
    enable_mold: bool,
) -> bool {
    let sccache_enabled = if enable_sccache {
        configure_sccache(cmd)
    } else {
        false
    };
    configure_mold(cmd, enable_mold);
    sccache_enabled
}

fn configure_sccache(cmd: &mut Command) -> bool {
    if env::var_os("RUSTC_WRAPPER").is_some() {
        warn!("RUSTC_WRAPPER already set; not overriding with sccache");
        return false;
    }

    which("sccache").map_or_else(
        |_| {
            warn!("`sccache` not found on PATH; proceeding without build cache");
            false
        },
        |path| {
            cmd.env("RUSTC_WRAPPER", path);
            true
        },
    )
}

#[cfg(target_os = "linux")]
fn configure_mold(cmd: &mut Command, enable: bool) {
    if !enable {
        return;
    }

    const MOLD_FLAG: &str = "-C";
    const MOLD_VALUE: &str = "link-arg=-fuse-ld=mold";

    let mut rustflags: Vec<String> = env::var("RUSTFLAGS")
        .ok()
        .map(|flags| flags.split_whitespace().map(ToString::to_string).collect())
        .unwrap_or_default();

    let already_set = rustflags
        .windows(2)
        .any(|win| win == [MOLD_FLAG, MOLD_VALUE]);

    if already_set {
        return;
    }

    rustflags.push(MOLD_FLAG.to_string());
    rustflags.push(MOLD_VALUE.to_string());
    let joined = rustflags.join(" ");
    cmd.env("RUSTFLAGS", joined);
}

#[cfg(not(target_os = "linux"))]
const fn configure_mold(_cmd: &mut Command, _enable: bool) {}
