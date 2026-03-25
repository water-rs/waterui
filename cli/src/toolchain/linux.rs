//! Linux system package toolchain checks.

use color_eyre::eyre;

use crate::{
    toolchain::{Installation, Toolchain, ToolchainError, UnfixableToolchain},
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

    /// Build an installation plan for explicit package names using the detected manager.
    ///
    /// This reuses the existing Linux package-manager framework and is useful for
    /// toolchains that discover missing capabilities via probes (for example,
    /// pkg-config modules).
    ///
    /// # Errors
    /// Returns an error when no supported package manager is available.
    pub async fn from_packages(packages: Vec<String>) -> Result<Self, UnfixableToolchain> {
        let Some(manager) = LinuxPackageManager::detect().await else {
            return Err(UnfixableToolchain::new(
                "Unable to detect Linux package manager",
                unsupported_manager_hint(),
            ));
        };
        if packages.is_empty() {
            return Err(UnfixableToolchain::new(
                "No packages were provided for automatic installation",
                "Re-run `water doctor` and inspect diagnostics.",
            ));
        }
        Ok(Self::new(manager, packages))
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
                "libgtk-4-dev",
                "libpango1.0-dev",
                "libwayland-dev",
                "wayland-protocols",
                "libasound2-dev",
                "libva-dev",
                "libgbm-dev",
                "libxcb1-dev",
                "libclang-dev",
                "libfontconfig-dev",
            ],
            Self::Dnf => &[
                "pkgconf-pkg-config",
                "gtk4-devel",
                "pango-devel",
                "wayland-devel",
                "wayland-protocols-devel",
                "alsa-lib-devel",
                "libva-devel",
                "mesa-libgbm-devel",
                "libxcb-devel",
                "clang-devel",
                "fontconfig-devel",
            ],
            Self::Pacman => &[
                "pkgconf",
                "gtk4",
                "pango",
                "wayland",
                "wayland-protocols",
                "alsa-lib",
                "libva",
                "mesa",
                "libxcb",
                "clang",
                "fontconfig",
            ],
            Self::Zypper => &[
                "pkg-config",
                "gtk4-devel",
                "pango-devel",
                "wayland-devel",
                "wayland-protocols-devel",
                "alsa-devel",
                "libva-devel",
                "Mesa-libgbm-devel",
                "libxcb-devel",
                "clang-devel",
                "fontconfig-devel",
            ],
            Self::Apk => &[
                "pkgconf",
                "gtk4.0-dev",
                "pango-dev",
                "wayland-dev",
                "wayland-protocols",
                "alsa-lib-dev",
                "libva-dev",
                "mesa-dev",
                "libxcb-dev",
                "clang-dev",
                "fontconfig-dev",
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

    fn package_for_gtk_pkg_config_probe(self, probe: &str) -> Option<&'static str> {
        if probe.starts_with("gtk4") {
            return Some(match self {
                Self::Apt => "libgtk-4-dev",
                Self::Dnf | Self::Zypper => "gtk4-devel",
                Self::Pacman => "gtk4",
                Self::Apk => "gtk4.0-dev",
            });
        }
        if probe.starts_with("pango") {
            return Some(match self {
                Self::Apt => "libpango1.0-dev",
                Self::Dnf | Self::Zypper => "pango-devel",
                Self::Pacman => "pango",
                Self::Apk => "pango-dev",
            });
        }
        None
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
    let alpine_hint = LinuxPackageManager::Apk.install_hint(&required_packages_to_owned(
        LinuxPackageManager::Apk.required_packages(),
    ));
    format!(
        "Install required packages manually. Debian/Ubuntu: `{apt_hint}`; Fedora/RHEL: `{dnf_hint}`; Arch: `{pacman_hint}`; openSUSE: `{zypper_hint}`; Alpine: `{alpine_hint}`."
    )
}

/// Returns `true` when a supported Linux package manager is available.
pub async fn has_supported_package_manager() -> bool {
    LinuxPackageManager::detect().await.is_some()
}

/// Build an installation plan that repairs missing GTK pkg-config probes.
///
/// Supported probe names include `gtk4` and `pango>=1.50`.
///
/// # Errors
/// Returns an error if no package manager is available or if a probe cannot be
/// mapped to an installable system package.
pub async fn gtk4_pkg_config_repair_installation(
    missing_modules: &[String],
) -> Result<LinuxSystemPackagesInstallation, UnfixableToolchain> {
    let Some(manager) = LinuxPackageManager::detect().await else {
        return Err(UnfixableToolchain::new(
            "Unable to detect Linux package manager",
            unsupported_manager_hint(),
        ));
    };

    let mut packages = Vec::new();
    for module in missing_modules {
        let package = manager
            .package_for_gtk_pkg_config_probe(module)
            .ok_or_else(|| {
                UnfixableToolchain::new(
                    format!("No package mapping is defined for GTK probe `{module}`"),
                    "Install a package that provides the missing module via pkg-config, then re-run `water doctor`.",
                )
            })?;
        if !packages.iter().any(|existing| existing == package) {
            packages.push(package.to_owned());
        }
    }

    LinuxSystemPackagesInstallation::from_packages(packages).await
}

/// Install named packages with the detected Linux package manager.
///
/// # Errors
/// Returns an error when no supported package manager is available, or when
/// installation fails.
pub async fn install_named_packages(packages: &[&'static str]) -> eyre::Result<()> {
    let Some(manager) = LinuxPackageManager::detect().await else {
        return Err(eyre::eyre!(
            "No supported Linux package manager found (apt-get, dnf, pacman, zypper, apk)."
        ));
    };

    let packages: Vec<String> = packages
        .iter()
        .map(|package| (*package).to_string())
        .collect();
    install_missing_packages(manager, &packages).await
}

/// Install a JDK package using the detected Linux package manager.
///
/// # Errors
/// Returns an error when no supported package manager is available, or when
/// installation fails.
pub async fn install_java_jdk() -> eyre::Result<()> {
    let Some(manager) = LinuxPackageManager::detect().await else {
        return Err(eyre::eyre!(
            "No supported Linux package manager found (apt-get, dnf, pacman, zypper, apk)."
        ));
    };

    let packages: Vec<String> = match manager {
        LinuxPackageManager::Apt => vec![String::from("openjdk-21-jdk")],
        LinuxPackageManager::Dnf | LinuxPackageManager::Zypper => {
            vec![String::from("java-21-openjdk-devel")]
        }
        LinuxPackageManager::Pacman => vec![String::from("jdk-openjdk")],
        LinuxPackageManager::Apk => vec![String::from("openjdk21-jdk")],
    };

    install_missing_packages(manager, &packages).await
}

#[cfg(test)]
mod tests {
    use super::LinuxPackageManager;

    #[test]
    fn dnf_packages_include_validated_core_deps() {
        let required = LinuxPackageManager::Dnf.required_packages();
        assert!(required.contains(&"gtk4-devel"));
        assert!(required.contains(&"pango-devel"));
        assert!(required.contains(&"wayland-devel"));
        assert!(required.contains(&"libva-devel"));
        assert!(required.contains(&"mesa-libgbm-devel"));
        assert!(required.contains(&"libxcb-devel"));
        assert!(required.contains(&"alsa-lib-devel"));
        assert!(required.contains(&"clang-devel"));
        assert!(required.contains(&"fontconfig-devel"));
    }

    #[test]
    fn apt_hint_uses_apt_get_install() {
        let hint = LinuxPackageManager::Apt
            .install_hint(&[String::from("libwayland-dev"), String::from("libva-dev")]);
        assert_eq!(hint, "sudo apt-get install -y libwayland-dev libva-dev");
    }

    #[test]
    fn apt_required_packages_include_gtk4_dev() {
        let required = LinuxPackageManager::Apt.required_packages();
        assert!(required.contains(&"libgtk-4-dev"));
        assert!(required.contains(&"libpango1.0-dev"));
    }

    #[test]
    fn pacman_hint_uses_needed_flag() {
        let hint = LinuxPackageManager::Pacman
            .install_hint(&[String::from("wayland"), String::from("libva")]);
        assert_eq!(hint, "sudo pacman -S --noconfirm --needed wayland libva");
    }

    #[test]
    fn dnf_probe_mapping_covers_pango_and_gtk4() {
        assert_eq!(
            LinuxPackageManager::Dnf.package_for_gtk_pkg_config_probe("gtk4"),
            Some("gtk4-devel")
        );
        assert_eq!(
            LinuxPackageManager::Dnf.package_for_gtk_pkg_config_probe("pango>=1.50"),
            Some("pango-devel")
        );
    }
}
