//! Shared helper for Windows package installation via winget.

use crate::{toolchain::Host, utils::CommandError};

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
    CommandFailed(#[from] CommandError),
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
pub async fn ensure_package_installed(
    host: &Host,
    package_id: &'static str,
) -> Result<(), WingetInstallError> {
    if host.which("winget").await.is_err() {
        return Err(WingetInstallError::WingetNotFound);
    }

    if is_package_installed(host, package_id).await? {
        return Ok(());
    }

    let mut args = vec!["install", "--id", package_id];
    args.extend(WINGET_COMMON_FLAGS);
    host.run("winget", args)
        .await
        .map_err(WingetInstallError::CommandFailed)?;

    if is_package_installed(host, package_id).await? {
        Ok(())
    } else {
        Err(WingetInstallError::NotInstalled { package_id })
    }
}

async fn is_package_installed(
    host: &Host,
    package_id: &'static str,
) -> Result<bool, WingetInstallError> {
    let output = host
        .output("winget", ["list", "--id", package_id, "--exact"])
        .await
        .map_err(WingetInstallError::CommandFailed)?;
    Ok(output.status.success())
}

#[cfg(test)]
mod tests {
    use super::{WingetInstallError, ensure_package_installed};
    use crate::toolchain::testing::TestMachine;

    #[test]
    fn winget_absent_reports_winget_not_found() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(ensure_package_installed(&host, "Kitware.CMake"));
        assert!(
            matches!(result, Err(WingetInstallError::WingetNotFound)),
            "no winget on PATH must report WingetNotFound: {result:?}"
        );
    }

    #[test]
    fn installed_package_skips_install() {
        let machine = TestMachine::new();
        machine.install("winget");
        let host = machine.host([(
            String::from("WATERUI_FAKE_WINGET_INSTALLED"),
            String::from("Kitware.CMake"),
        )]);
        smol::block_on(ensure_package_installed(&host, "Kitware.CMake"))
            .expect("an installed package must short-circuit to ok");
    }

    #[test]
    fn package_still_missing_after_install_is_an_error() {
        let machine = TestMachine::new();
        machine.install("winget");
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(ensure_package_installed(&host, "Kitware.CMake"));
        assert!(
            matches!(result, Err(WingetInstallError::NotInstalled { .. })),
            "a package that never lists must surface NotInstalled: {result:?}"
        );
    }
}
