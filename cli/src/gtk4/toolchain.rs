//! GTK4 toolchain checking.

use smol::process::Command;

use crate::toolchain::{
    Toolchain, ToolchainError, UnfixableToolchain,
    linux::{LinuxSystemPackagesInstallation, LinuxSystemToolchain},
};

/// GTK4 toolchain checker.
///
/// Verifies that GTK4 development libraries are installed on Linux.
#[derive(Debug, Clone, Copy, Default)]
pub struct Gtk4Toolchain;

impl Toolchain for Gtk4Toolchain {
    type Installation = LinuxSystemPackagesInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if !cfg!(target_os = "linux") {
            return Err(ToolchainError::Unfixable(UnfixableToolchain::new(
                "GTK4 backend is only supported on Linux",
                "Run GTK4 targets on Linux with `--platform linux --backend gtk4`.",
            )));
        }

        if check_pkg_config_exists().await && check_gtk4_exists().await {
            return Ok(());
        }

        let linux_toolchain = LinuxSystemToolchain;
        return match linux_toolchain.check().await {
            Ok(()) => Err(ToolchainError::Unfixable(UnfixableToolchain::new(
                "GTK4 development libraries not found",
                install_gtk4_suggestion(),
            ))),
            Err(ToolchainError::Fixable(installation)) => {
                Err(ToolchainError::Fixable(installation))
            }
            Err(ToolchainError::Unfixable(e)) => Err(ToolchainError::Unfixable(e)),
        };
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

/// Get platform-specific suggestion for installing GTK4.
fn install_gtk4_suggestion() -> &'static str {
    "GTK4 was not discoverable via pkg-config. Ensure GTK4 development packages are installed for your distribution and `pkg-config --exists gtk4` succeeds."
}

#[cfg(test)]
mod tests {
    use super::install_gtk4_suggestion;

    #[test]
    fn gtk4_suggestion_mentions_pkg_config_probe() {
        let gtk4 = install_gtk4_suggestion();
        assert!(gtk4.contains("pkg-config"));
        assert!(gtk4.contains("gtk4"));
    }
}
