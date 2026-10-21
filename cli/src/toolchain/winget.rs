//! Shared helper for Windows package installation via winget.

use std::time::Duration;

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
    "--disable-interactivity",
    "--accept-package-agreements",
    "--accept-source-agreements",
];

/// How long one `winget` call may run before it is killed.
#[derive(Debug, Clone, Copy)]
struct Deadlines {
    /// `winget install`.
    install: Duration,
    /// `winget list`.
    probe: Duration,
}

/// The bounds `ensure_package_installed` applies.
///
/// `install` downloads an installer and runs it silently; the packages
/// `doctor --fix` asks for are tens of megabytes and install in well under a
/// minute even on a cold CI runner, so an install still running after a
/// quarter of an hour is hung, not slow (a nightly fresh-user run sat 3h43m
/// in `winget install Mozilla.sccache`). `list` only reads the local package
/// database and answers in seconds; two minutes covers a cold source
/// refresh.
const DEADLINES: Deadlines = Deadlines {
    install: Duration::from_mins(15),
    probe: Duration::from_mins(2),
};

/// Install a package via winget if it is not already installed.
pub async fn ensure_package_installed(
    host: &Host,
    package_id: &'static str,
) -> Result<(), WingetInstallError> {
    ensure_package_installed_within(host, package_id, DEADLINES).await
}

async fn ensure_package_installed_within(
    host: &Host,
    package_id: &'static str,
    deadlines: Deadlines,
) -> Result<(), WingetInstallError> {
    if host.which("winget").await.is_err() {
        return Err(WingetInstallError::WingetNotFound);
    }

    if is_package_installed(host, package_id, deadlines.probe).await? {
        return Ok(());
    }

    let mut args = vec!["install", "--id", package_id];
    args.extend(WINGET_COMMON_FLAGS);
    host.run_within("winget", args, deadlines.install)
        .await
        .map_err(WingetInstallError::CommandFailed)?;

    if is_package_installed(host, package_id, deadlines.probe).await? {
        Ok(())
    } else {
        Err(WingetInstallError::NotInstalled { package_id })
    }
}

async fn is_package_installed(
    host: &Host,
    package_id: &'static str,
    deadline: Duration,
) -> Result<bool, WingetInstallError> {
    let output = host
        .output_within(
            "winget",
            [
                "list",
                "--id",
                package_id,
                "--exact",
                "--disable-interactivity",
            ],
            deadline,
        )
        .await
        .map_err(WingetInstallError::CommandFailed)?;
    Ok(output.status.success())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        Deadlines, WingetInstallError, ensure_package_installed, ensure_package_installed_within,
    };
    use crate::{toolchain::testing::TestMachine, utils::CommandError};

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

    /// An install that never returns is killed at its deadline and reported
    /// as `TimedOut`; the fake's child dies with the tree, so nothing is
    /// left running.
    #[test]
    fn hung_install_times_out() {
        let machine = TestMachine::new();
        machine.install("winget");
        let host = machine.host([(String::from("WATERUI_FAKE_WINGET_HANG"), String::from("1"))]);
        let deadlines = Deadlines {
            install: Duration::from_millis(500),
            probe: Duration::from_mins(1),
        };
        let result = smol::block_on(ensure_package_installed_within(
            &host,
            "Kitware.CMake",
            deadlines,
        ));
        assert!(
            matches!(
                result,
                Err(WingetInstallError::CommandFailed(
                    CommandError::TimedOut { .. }
                ))
            ),
            "a hung install must surface TimedOut: {result:?}"
        );
    }
}
