//! Toolchain support for `git`.
//!
//! `water create` initializes the scaffolded project as a git repository, so
//! `git` is a required tool on every supported host — not just a build
//! dependency of one backend. The check is a plain PATH probe; the repair is
//! the host's own package installer (`brew`, `winget`, or the distribution's
//! package manager), matching how every distribution the CLI runs on ships
//! git.

use std::{
    ffi::{OsStr, OsString},
    io,
    process::Output,
};

use smol::process::Command;

use crate::{
    brew::Brew,
    toolchain::linux::{
        LinuxPackageManagerError, has_supported_package_manager, install_named_packages,
    },
    toolchain::winget::{WingetInstallError, ensure_package_installed},
    toolchain::{Host, Installation, Toolchain, ToolchainError},
    utils::CommandError,
};

/// The `-c` config switches every `git` the CLI spawns runs under.
///
/// `core.longpaths=true` opts each invocation into paths past `MAX_PATH`: the
/// build cache nests a `drive-<letter>/…` copy of the project's absolute
/// path under `~/.water`, and managed checkouts add
/// `managed_backends/…/<sha>/` below it, crossing 260 characters on Windows
/// (water-rs/waterui#3125). Git honours long paths only when the machine
/// enables them — the `windows-long-paths` doctor item checks that setting.
pub const CONFIG_ARGS: &[&str] = &["-c", "core.longpaths=true"];

/// `args` with [`CONFIG_ARGS`] in front, as [`Host::run`] and
/// [`Host::output`] take them.
fn argv(args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Vec<OsString> {
    CONFIG_ARGS
        .iter()
        .map(|arg| OsString::from(*arg))
        .chain(args.into_iter().map(|arg| arg.as_ref().to_os_string()))
        .collect()
}

/// A `git` [`Command`] on `host` with [`CONFIG_ARGS`] applied; the caller
/// appends the subcommand and its arguments.
///
/// # Errors
/// - [`io::ErrorKind::NotFound`] when `git` is not on the host's `PATH`.
pub fn command(host: &Host) -> io::Result<Command> {
    let mut command = host.command("git")?;
    command.args(CONFIG_ARGS);
    Ok(command)
}

/// [`command`] for the synchronous call sites.
///
/// # Errors
/// - [`io::ErrorKind::NotFound`] when `git` is not on the host's `PATH`.
pub fn std_command(host: &Host) -> io::Result<std::process::Command> {
    let mut command = host.std_command("git")?;
    command.args(CONFIG_ARGS);
    Ok(command)
}

/// `git` `args` on `host` under [`CONFIG_ARGS`]; [`Host::run`]'s contract.
///
/// # Errors
/// Every error [`Host::run`] reports.
pub async fn run(
    host: &Host,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
) -> Result<String, CommandError> {
    host.run("git", argv(args)).await
}

/// [`run`] returning the full [`Output`]; [`Host::output`]'s contract.
///
/// # Errors
/// Every error [`Host::output`] reports.
pub async fn output(
    host: &Host,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
) -> Result<Output, CommandError> {
    host.output("git", argv(args)).await
}

/// Toolchain for `git`.
#[derive(Debug, Clone, Default)]
pub struct Git;

impl Toolchain for Git {
    type Installation = GitInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if host.which("git").await.is_ok() {
            Ok(())
        } else if cfg!(target_os = "windows") {
            if host.which("winget").await.is_ok() {
                Err(ToolchainError::fixable(GitInstallation::Winget))
            } else {
                Err(ToolchainError::unfixable(
                    "git is missing and winget is unavailable",
                    "Install Git for Windows from https://git-scm.com/download/win and ensure `git` is available in PATH.",
                ))
            }
        } else if cfg!(target_os = "macos") {
            if host.which("brew").await.is_ok() {
                Err(ToolchainError::fixable(GitInstallation::Brew))
            } else {
                Err(ToolchainError::unfixable(
                    "git is missing and Homebrew is unavailable",
                    "Install the Xcode Command Line Tools (`xcode-select --install`) or Git from https://git-scm.com/download/mac, then re-run `water doctor`.",
                ))
            }
        } else if cfg!(target_os = "linux") {
            if has_supported_package_manager(host).await {
                Err(ToolchainError::fixable(GitInstallation::PackageManager))
            } else {
                Err(ToolchainError::unfixable(
                    "git is missing and no supported package manager was found",
                    "Install git with your distribution's package manager and ensure `git` is available in PATH.",
                ))
            }
        } else {
            Err(ToolchainError::unfixable(
                "git not found",
                "Install git for your platform and ensure `git` is available in PATH.",
            ))
        }
    }
}

/// Installation plan for `git` — the strategy `check` selected for this host.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum GitInstallation {
    /// `brew install git`.
    Brew,
    /// `winget install Git.Git`.
    Winget,
    /// The host's Linux package manager.
    PackageManager,
}

/// Errors that can occur during `git` installation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FailToInstallGit {
    /// Homebrew not found error.
    #[error("Homebrew not found. Please install Homebrew to proceed.")]
    BrewNotFound,

    /// An installation command failed.
    #[error("Failed to install git: {0}")]
    Command(#[from] CommandError),

    /// winget is required for Windows automatic installation.
    #[error(
        "winget is required for automatic git installation on Windows. Install App Installer and retry."
    )]
    WingetNotFound,

    /// Windows installation via winget failed.
    #[error("Failed to install git via winget: {0}")]
    WingetInstallFailed(String),

    /// Linux package manager is required for automatic installation.
    #[error(
        "No supported Linux package manager found (apt-get, dnf, pacman, zypper, apk). Install git manually."
    )]
    UnsupportedPackageManager,
}

impl Installation for GitInstallation {
    type Error = FailToInstallGit;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        match self {
            Self::Brew => {
                let brew = Brew::default();
                brew.check(host)
                    .await
                    .map_err(|_| FailToInstallGit::BrewNotFound)?;
                brew.install(host, "git").await?;
                Ok(())
            }
            Self::Winget => ensure_package_installed(host, "Git.Git")
                .await
                .map_err(map_winget_error_for_git),
            Self::PackageManager => install_named_packages(host, &["git"])
                .await
                .map_err(map_linux_error_for_git),
        }
    }
}

fn map_linux_error_for_git(error: LinuxPackageManagerError) -> FailToInstallGit {
    match error {
        LinuxPackageManagerError::UnsupportedPackageManager => {
            FailToInstallGit::UnsupportedPackageManager
        }
        LinuxPackageManagerError::Command(source) => FailToInstallGit::Command(source),
    }
}

fn map_winget_error_for_git(error: WingetInstallError) -> FailToInstallGit {
    match error {
        WingetInstallError::WingetNotFound => FailToInstallGit::WingetNotFound,
        WingetInstallError::CommandFailed(err) => {
            FailToInstallGit::WingetInstallFailed(err.to_string())
        }
        WingetInstallError::NotInstalled { package_id } => {
            FailToInstallGit::WingetInstallFailed(format!(
                "Package `{package_id}` is still missing after winget install; verify winget sources and retry."
            ))
        }
    }
}

#[cfg(test)]
mod host_tests {
    use super::{Git, GitInstallation};
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Installation, Toolchain, ToolchainError};

    fn check(machine: &TestMachine) -> Result<(), ToolchainError<GitInstallation>> {
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Git.check(&host))
    }

    #[test]
    fn ok_when_git_on_path() {
        let machine = TestMachine::new();
        machine.install("git");
        check(&machine).expect("git on PATH must be ok");
    }

    /// The fake tools log `<tool> <args>` per invocation under
    /// `WATERUI_FAKE_LOG` — every `git` the CLI spawns must carry
    /// `-c core.longpaths=true` ahead of its subcommand so a Windows host
    /// with `LongPathsEnabled` checks out paths past `MAX_PATH`.
    #[test]
    fn invocations_carry_longpaths_config() {
        let machine = TestMachine::new();
        machine.install("git");
        let log = machine.file("git.log", "");
        let host = machine.host([("WATERUI_FAKE_LOG", log.as_os_str())]);
        smol::block_on(super::run(&host, ["status"])).expect("fake git run");
        smol::block_on(
            super::command(&host)
                .expect("the fake git is on the host's PATH")
                .arg("status")
                .status(),
        )
        .expect("fake git command status");
        super::std_command(&host)
            .expect("the fake git is on the host's PATH")
            .arg("status")
            .status()
            .expect("fake git std_command status");
        let logged = std::fs::read_to_string(&log).expect("fake tool log");
        assert_eq!(
            logged.lines().collect::<Vec<_>>(),
            [
                "git -c core.longpaths=true status",
                "git -c core.longpaths=true status",
                "git -c core.longpaths=true status",
            ]
        );
    }

    #[test]
    fn missing_without_installer_is_unfixable() {
        let machine = TestMachine::new();
        let result = check(&machine);
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "missing git without a package manager must be unfixable: {result:?}"
        );
    }

    #[test]
    fn missing_with_installer_is_fixable() {
        let machine = TestMachine::new();
        #[cfg(target_os = "macos")]
        machine.install("brew");
        #[cfg(target_os = "linux")]
        machine.install("apt-get");
        #[cfg(target_os = "windows")]
        machine.install("winget");
        let result = check(&machine);
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing git with a package manager must be fixable: {result:?}"
        );
    }

    /// The fake `winget` accepts `install` but never reports the package
    /// afterwards, so the post-install verification must fail fast instead of
    /// reporting success — the same contract `cmake` is held to.
    #[test]
    #[cfg(target_os = "windows")]
    fn install_fails_when_winget_leaves_the_package_missing() {
        let machine = TestMachine::new();
        machine.install("winget");
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(GitInstallation::Winget.install(&host));
        assert!(
            matches!(result, Err(super::FailToInstallGit::WingetInstallFailed(_))),
            "a package still missing after winget install must be an error: {result:?}"
        );
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn install_runs_the_package_manager() {
        let machine = TestMachine::new();
        #[cfg(target_os = "macos")]
        machine.install("brew");
        #[cfg(target_os = "linux")]
        machine.install("apt-get");
        let host = machine.host(Vec::<(String, String)>::new());
        let installation = if cfg!(target_os = "macos") {
            GitInstallation::Brew
        } else {
            GitInstallation::PackageManager
        };
        smol::block_on(installation.install(&host))
            .expect("installing git through the host's package manager must succeed");
    }
}
