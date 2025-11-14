use std::{path::Path, process::Command};

use color_eyre::eyre::Result;

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
