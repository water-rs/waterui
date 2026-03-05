//! Shared helper for Windows package installation via winget.

use color_eyre::eyre;

use crate::utils::{run_command, run_command_output_os, which};

/// Errors from winget-backed installation.
#[derive(Debug, thiserror::Error)]
pub enum WingetInstallError {
    /// winget is not available on this host.
    #[error(
        "winget is not available. Install Microsoft App Installer, then retry `water doctor --fix`."
    )]
    WingetNotFound,
    /// Failed to invoke winget commands.
    #[error("winget command failed: {0}")]
    CommandFailed(#[from] eyre::Report),
    /// Package still not detected after install command succeeded.
    #[error("winget install completed but package `{package_id}` is still not detected")]
    NotInstalled { package_id: &'static str },
}

const WINGET_COMMON_FLAGS: &[&str] = &[
    "--exact",
    "--silent",
    "--accept-package-agreements",
    "--accept-source-agreements",
];

/// Install a package via winget if it is not already installed.
pub async fn ensure_package_installed(package_id: &'static str) -> Result<(), WingetInstallError> {
    if which("winget").await.is_err() {
        return Err(WingetInstallError::WingetNotFound);
    }

    if is_package_installed(package_id).await? {
        return Ok(());
    }

    let mut args = vec!["install", "--id", package_id];
    args.extend(WINGET_COMMON_FLAGS);
    run_command("winget", args)
        .await
        .map_err(WingetInstallError::CommandFailed)?;

    if is_package_installed(package_id).await? {
        Ok(())
    } else {
        Err(WingetInstallError::NotInstalled { package_id })
    }
}

async fn is_package_installed(package_id: &'static str) -> Result<bool, WingetInstallError> {
    let output = run_command_output_os("winget", ["list", "--id", package_id, "--exact"])
        .await
        .map_err(WingetInstallError::CommandFailed)?;
    Ok(output.status.success())
}
