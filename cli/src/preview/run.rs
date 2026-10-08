//! Shared helpers for executing a generated preview binary.
//!
//! A preview binary — the managed Hydrolysis backend's or the Apple preview
//! package's — reads a [`PreviewRunConfig`] JSON from the path its
//! `WATERUI_PREVIEW_RUN_CONFIG` environment variable names. These helpers
//! write that file, exec the binary in the generated crate's directory, and
//! check the output the run was asked to produce.

use std::path::{Path, PathBuf};
use std::process::Output;

use eyre::{Context as _, Result, bail};

use crate::utils::command;

use waterui_preview_protocol::run::{PREVIEW_RUN_CONFIG_ENV, PreviewRunConfig};

/// Writes the run config JSON next to the backend sources and returns its
/// path; the file is overwritten per invocation.
pub async fn write_run_config(dir: &Path, config: &PreviewRunConfig) -> Result<PathBuf> {
    let path = dir.join("preview-run.json");
    let json =
        serde_json::to_vec_pretty(config).wrap_err("Failed to serialize preview run config")?;
    smol::fs::write(&path, json)
        .await
        .wrap_err_with(|| format!("Failed to write {}", path.display()))?;
    Ok(path)
}

/// Spawns the preview binary in `working_dir` with
/// `WATERUI_PREVIEW_RUN_CONFIG` pointing at `run_config_path` and returns
/// its [`Output`].
///
/// `label` names the binary in errors: "Apple preview", "Hydrolysis
/// preview", "Hydrolysis preview test".
///
/// # Errors
/// Returns an error when the binary cannot be spawned or exits non-zero —
/// the error carries its stderr, else its stdout, else the exit status.
pub async fn run_preview_binary(
    host: &crate::toolchain::Host,
    working_dir: &Path,
    binary_path: &Path,
    run_config_path: &Path,
    label: &str,
) -> Result<Output> {
    let mut child = host.command(binary_path);
    let child = command(&mut child);
    child.current_dir(working_dir);
    child.env(PREVIEW_RUN_CONFIG_ENV, run_config_path);

    let output = child
        .output()
        .await
        .wrap_err_with(|| format!("Failed to run {label} binary {}", binary_path.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let details = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit status {}", output.status)
        };
        bail!("{label} binary failed: {details}");
    }
    Ok(output)
}

/// `path` as an absolute path, resolved against the current directory.
pub fn absolute_output_path(host: &crate::toolchain::Host, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    host.cwd().join(path)
}

/// Checks the preview wrote a non-empty `what` at `path`.
///
/// # Errors
/// Returns an error when `path` is missing or empty.
pub async fn expect_nonempty_output(path: &Path, what: &str) -> Result<()> {
    let metadata = smol::fs::metadata(path)
        .await
        .wrap_err_with(|| format!("preview did not produce {what} {}", path.display()))?;
    if metadata.len() == 0 {
        bail!("preview wrote empty {what} to {}", path.display());
    }
    Ok(())
}
