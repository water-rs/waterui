//! Linux system package toolchain checks.

use color_eyre::eyre;

use crate::{
    toolchain::{Installation, Toolchain, ToolchainError},
    utils::{run_command, run_command_output_os, which},
};

/// Linux system dependencies required by `waterui` desktop/media builds.
#[derive(Debug, Clone, Copy, Default)]
pub struct LinuxSystemToolchain;

/// Installation plan for missing Linux system packages.
#[derive(Debug, Clone)]
pub struct LinuxSystemPackagesInstallation {
    manager: LinuxPackageManager,
    missing_packages: Vec<String>,
}

impl LinuxSystemPackagesInstallation {
    const fn new(manager: LinuxPackageManager, missing_packages: Vec<String>) -> Self {
        Self {
            manager,
            missing_packages,
        }
    }

    /// Returns the detected package manager name.
    #[must_use]
    pub const fn package_manager_name(&self) -> &'static str {
        self.manager.name()
    }

    /// Returns the missing packages for this installation plan.
    #[must_use]
    pub fn missing_packages(&self) -> &[String] {
        &self.missing_packages
    }

    /// Returns a command hint for manual installation.
    #[must_use]
    pub fn install_command_hint(&self) -> String {
        self.manager.install_hint(&self.missing_packages)
    }
}

/// Errors that can occur during Linux package installation.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallLinuxSystemPackages {
    /// Non-Linux platforms are not supported by this installer.
    #[error("Automatic Linux package installation is only supported on Linux hosts.")]
    UnsupportedPlatform,
    /// No supported package manager was detected.
    #[error("No supported Linux package manager found (apt-get, dnf, pacman, zypper, apk).")]
    UnsupportedPackageManager,
    /// Package installation failed.
    #[error("Failed to install Linux system packages: {0}")]
    Other(eyre::Report),
}

impl Installation for LinuxSystemPackagesInstallation {
    type Error = FailToInstallLinuxSystemPackages;

    async fn install(&self) -> Result<(), Self::Error> {
        if !cfg!(target_os = "linux") {
            return Err(FailToInstallLinuxSystemPackages::UnsupportedPlatform);
        }

        let Some(manager) = LinuxPackageManager::detect().await else {
            return Err(FailToInstallLinuxSystemPackages::UnsupportedPackageManager);
        };

        install_missing_packages(manager, &self.missing_packages)
            .await
            .map_err(FailToInstallLinuxSystemPackages::Other)
    }
}

impl Toolchain for LinuxSystemToolchain {
    type Installation = LinuxSystemPackagesInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if !cfg!(target_os = "linux") {
            return Ok(());
        }

        let Some(manager) = LinuxPackageManager::detect().await else {
            return Err(ToolchainError::unfixable(
                "Unable to detect Linux package manager",
                unsupported_manager_hint(),
            ));
        };

        let required_packages = manager.required_packages();
        let mut missing_packages = Vec::new();
        for &package in required_packages {
            let installed = manager.check_installed(package).await.map_err(|error| {
                ToolchainError::unfixable(
                    format!("Failed checking Linux package `{package}`: {error}"),
                    manager.install_hint(&required_packages_to_owned(required_packages)),
                )
            })?;
            if !installed {
                missing_packages.push(package.to_string());
            }
        }

        if missing_packages.is_empty() {
            Ok(())
        } else {
            Err(ToolchainError::fixable(
                LinuxSystemPackagesInstallation::new(manager, missing_packages),
            ))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinuxPackageManager {
    Apt,
    Dnf,
    Pacman,
    Zypper,
    Apk,
}

impl LinuxPackageManager {
    async fn detect() -> Option<Self> {
        if which("apt-get").await.is_ok() {
            Some(Self::Apt)
        } else if which("dnf").await.is_ok() {
            Some(Self::Dnf)
        } else if which("pacman").await.is_ok() {
            Some(Self::Pacman)
        } else if which("zypper").await.is_ok() {
            Some(Self::Zypper)
        } else if which("apk").await.is_ok() {
            Some(Self::Apk)
        } else {
            None
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Apt => "apt-get",
            Self::Dnf => "dnf",
            Self::Pacman => "pacman",
            Self::Zypper => "zypper",
            Self::Apk => "apk",
        }
    }

    const fn required_packages(self) -> &'static [&'static str] {
        match self {
            Self::Apt => &[
                "pkg-config",
                "libwayland-dev",
                "wayland-protocols",
                "libasound2-dev",
                "libva-dev",
                "libgbm-dev",
                "libxcb1-dev",
                "libclang-dev",
            ],
            Self::Dnf => &[
                "pkgconf-pkg-config",
                "wayland-devel",
                "wayland-protocols-devel",
                "alsa-lib-devel",
                "libva-devel",
                "mesa-libgbm-devel",
                "libxcb-devel",
                "clang-devel",
            ],
            Self::Pacman => &[
                "pkgconf",
                "wayland",
                "wayland-protocols",
                "alsa-lib",
                "libva",
                "mesa",
                "libxcb",
                "clang",
            ],
            Self::Zypper => &[
                "pkg-config",
                "wayland-devel",
                "wayland-protocols-devel",
                "alsa-devel",
                "libva-devel",
                "Mesa-libgbm-devel",
                "libxcb-devel",
                "clang-devel",
            ],
            Self::Apk => &[
                "pkgconf",
                "wayland-dev",
                "wayland-protocols",
                "alsa-lib-dev",
                "libva-dev",
                "mesa-dev",
                "libxcb-dev",
                "clang-dev",
            ],
        }
    }

    fn install_hint(self, packages: &[String]) -> String {
        let package_list = packages.join(" ");
        match self {
            Self::Apt => format!("sudo apt-get install -y {package_list}"),
            Self::Dnf => format!("sudo dnf install -y {package_list}"),
            Self::Pacman => format!("sudo pacman -S --noconfirm --needed {package_list}"),
            Self::Zypper => {
                format!(
                    "sudo zypper --non-interactive install --auto-agree-with-licenses {package_list}"
                )
            }
            Self::Apk => format!("sudo apk add {package_list}"),
        }
    }

    async fn check_installed(self, package: &str) -> eyre::Result<bool> {
        let output = match self {
            Self::Apt => run_command_output_os("dpkg-query", ["-W", package]).await?,
            Self::Dnf | Self::Zypper => run_command_output_os("rpm", ["-q", package]).await?,
            Self::Pacman => run_command_output_os("pacman", ["-Q", package]).await?,
            Self::Apk => run_command_output_os("apk", ["info", "-e", package]).await?,
        };
        Ok(output.status.success())
    }
}

async fn run_with_optional_sudo(command: &str, args: &[String]) -> eyre::Result<()> {
    if which("sudo").await.is_ok() {
        let mut sudo_args = Vec::with_capacity(args.len() + 1);
        sudo_args.push(command.to_string());
        sudo_args.extend(args.iter().cloned());
        run_command("sudo", sudo_args.iter().map(String::as_str)).await?;
    } else {
        run_command(command, args.iter().map(String::as_str)).await?;
    }
    Ok(())
}

async fn install_missing_packages(
    manager: LinuxPackageManager,
    packages: &[String],
) -> eyre::Result<()> {
    if packages.is_empty() {
        return Ok(());
    }

    match manager {
        LinuxPackageManager::Apt => {
            run_with_optional_sudo("apt-get", &[String::from("update")]).await?;
            let mut args = vec![String::from("install"), String::from("-y")];
            args.extend(packages.iter().cloned());
            run_with_optional_sudo("apt-get", &args).await?;
        }
        LinuxPackageManager::Dnf => {
            let mut args = vec![String::from("install"), String::from("-y")];
            args.extend(packages.iter().cloned());
            run_with_optional_sudo("dnf", &args).await?;
        }
        LinuxPackageManager::Pacman => {
            let mut args = vec![
                String::from("-S"),
                String::from("--noconfirm"),
                String::from("--needed"),
            ];
            args.extend(packages.iter().cloned());
            run_with_optional_sudo("pacman", &args).await?;
        }
        LinuxPackageManager::Zypper => {
            let mut args = vec![
                String::from("--non-interactive"),
                String::from("install"),
                String::from("--auto-agree-with-licenses"),
            ];
            args.extend(packages.iter().cloned());
            run_with_optional_sudo("zypper", &args).await?;
        }
        LinuxPackageManager::Apk => {
            let mut args = vec![String::from("add")];
            args.extend(packages.iter().cloned());
            run_with_optional_sudo("apk", &args).await?;
        }
    }

    Ok(())
}

fn required_packages_to_owned(packages: &[&str]) -> Vec<String> {
    packages
        .iter()
        .map(|package| (*package).to_string())
        .collect()
}

fn unsupported_manager_hint() -> String {
    let apt_hint = LinuxPackageManager::Apt.install_hint(&required_packages_to_owned(
        LinuxPackageManager::Apt.required_packages(),
    ));
    let dnf_hint = LinuxPackageManager::Dnf.install_hint(&required_packages_to_owned(
        LinuxPackageManager::Dnf.required_packages(),
    ));
    let pacman_hint = LinuxPackageManager::Pacman.install_hint(&required_packages_to_owned(
        LinuxPackageManager::Pacman.required_packages(),
    ));
    let zypper_hint = LinuxPackageManager::Zypper.install_hint(&required_packages_to_owned(
        LinuxPackageManager::Zypper.required_packages(),
    ));
    let apk_hint = LinuxPackageManager::Apk.install_hint(&required_packages_to_owned(
        LinuxPackageManager::Apk.required_packages(),
    ));
    format!(
        "Install required packages manually. Debian/Ubuntu: `{apt_hint}`; Fedora/RHEL: `{dnf_hint}`; Arch: `{pacman_hint}`; openSUSE: `{zypper_hint}`; Alpine: `{apk_hint}`."
    )
}

#[cfg(test)]
mod tests {
    use super::LinuxPackageManager;

    #[test]
    fn dnf_packages_include_validated_core_deps() {
        let required = LinuxPackageManager::Dnf.required_packages();
        assert!(required.contains(&"wayland-devel"));
        assert!(required.contains(&"libva-devel"));
        assert!(required.contains(&"mesa-libgbm-devel"));
        assert!(required.contains(&"libxcb-devel"));
        assert!(required.contains(&"alsa-lib-devel"));
        assert!(required.contains(&"clang-devel"));
    }

    #[test]
    fn apt_hint_uses_apt_get_install() {
        let hint = LinuxPackageManager::Apt
            .install_hint(&[String::from("libwayland-dev"), String::from("libva-dev")]);
        assert_eq!(hint, "sudo apt-get install -y libwayland-dev libva-dev");
    }

    #[test]
    fn pacman_hint_uses_needed_flag() {
        let hint = LinuxPackageManager::Pacman
            .install_hint(&[String::from("wayland"), String::from("libva")]);
        assert_eq!(hint, "sudo pacman -S --noconfirm --needed wayland libva");
    }
}
