//! GTK4 toolchain checking.

use color_eyre::eyre;
use smol::process::Command;

use crate::toolchain::{Installation, Toolchain, ToolchainError, UnfixableToolchain};

/// GTK4 toolchain checker.
///
/// Verifies that GTK4 development libraries are installed on the system.
#[derive(Debug, Clone, Copy, Default)]
pub struct Gtk4Toolchain;

/// Installation plan for GTK4 dependencies.
#[derive(Debug)]
pub enum Gtk4Installation {
    /// Install pkg-config via Homebrew (macOS).
    PkgConfig,
    /// Install GTK4 via Homebrew (macOS).
    Gtk4,
    /// Install both pkg-config and GTK4.
    Both,
}

impl Installation for Gtk4Installation {
    type Error = eyre::Report;

    async fn install(&self) -> Result<(), Self::Error> {
        match self {
            Self::PkgConfig => {
                crate::utils::run_command("brew", ["install", "pkg-config"]).await?;
            }
            Self::Gtk4 => {
                crate::utils::run_command("brew", ["install", "gtk4"]).await?;
            }
            Self::Both => {
                crate::utils::run_command("brew", ["install", "pkg-config", "gtk4"]).await?;
            }
        }
        Ok(())
    }
}

impl Toolchain for Gtk4Toolchain {
    type Installation = Gtk4Installation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        let pkg_config_ok = check_pkg_config_exists().await;
        let gtk4_ok = if pkg_config_ok {
            check_gtk4_exists().await
        } else {
            false
        };

        match (pkg_config_ok, gtk4_ok) {
            (true, true) => Ok(()),
            (false, _) if cfg!(target_os = "macos") => {
                // On macOS, we can install both via brew
                // Check if gtk4 would be available after pkg-config is installed
                Err(ToolchainError::Fixable(Gtk4Installation::Both))
            }
            (true, false) if cfg!(target_os = "macos") => {
                // pkg-config available, just missing GTK4
                Err(ToolchainError::Fixable(Gtk4Installation::Gtk4))
            }
            (false, _) => Err(ToolchainError::Unfixable(UnfixableToolchain::new(
                "pkg-config not found",
                install_pkg_config_suggestion(),
            ))),
            (true, false) => Err(ToolchainError::Unfixable(UnfixableToolchain::new(
                "GTK4 development libraries not found",
                install_gtk4_suggestion(),
            ))),
        }
    }
}

/// Check if pkg-config is available.
async fn check_pkg_config_exists() -> bool {
    Command::new("pkg-config")
        .arg("--version")
        .output()
        .await
        .is_ok_and(|o| o.status.success())
}

/// Check if GTK4 is installed via pkg-config.
async fn check_gtk4_exists() -> bool {
    Command::new("pkg-config")
        .args(["--exists", "gtk4"])
        .status()
        .await
        .is_ok_and(|s| s.success())
}

/// Get platform-specific suggestion for installing pkg-config.
fn install_pkg_config_suggestion() -> &'static str {
    if cfg!(target_os = "macos") {
        "Install pkg-config with Homebrew: brew install pkg-config"
    } else if cfg!(target_os = "linux") {
        "Install pkg-config with your package manager:\n\
         - Ubuntu/Debian: sudo apt install pkg-config\n\
         - Fedora: sudo dnf install pkgconfig\n\
         - Arch: sudo pacman -S pkgconf"
    } else if cfg!(target_os = "windows") {
        "Install pkg-config via MSYS2 or vcpkg"
    } else {
        "Install pkg-config for your platform"
    }
}

/// Get platform-specific suggestion for installing GTK4.
fn install_gtk4_suggestion() -> &'static str {
    if cfg!(target_os = "macos") {
        "Install GTK4 with Homebrew: brew install gtk4"
    } else if cfg!(target_os = "linux") {
        "Install GTK4 development libraries with your package manager:\n\
         - Ubuntu/Debian: sudo apt install libgtk-4-dev\n\
         - Fedora: sudo dnf install gtk4-devel\n\
         - Arch: sudo pacman -S gtk4"
    } else if cfg!(target_os = "windows") {
        "Install GTK4 via MSYS2:\n\
         pacman -S mingw-w64-x86_64-gtk4"
    } else {
        "Install GTK4 development libraries for your platform"
    }
}
