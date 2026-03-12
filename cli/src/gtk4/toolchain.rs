//! GTK4 toolchain checking.

use smol::process::Command;

use crate::toolchain::{
    Toolchain, ToolchainError, UnfixableToolchain,
    linux::{
        LinuxSystemPackagesInstallation, LinuxSystemToolchain, gtk4_pkg_config_repair_installation,
    },
};

/// GTK4 toolchain checker.
///
/// Verifies that GTK4 development libraries are installed on Linux.
#[derive(Debug, Clone, Copy, Default)]
pub struct Gtk4Toolchain;

#[derive(Debug, Clone, Copy)]
struct PkgConfigProbe {
    module: &'static str,
    min_version: Option<&'static str>,
}

impl PkgConfigProbe {
    fn display(self) -> String {
        match self.min_version {
            Some(min) => format!("{}>={min}", self.module),
            None => self.module.to_owned(),
        }
    }
}

const REQUIRED_PROBES: &[PkgConfigProbe] = &[
    PkgConfigProbe {
        module: "gtk4",
        min_version: None,
    },
    PkgConfigProbe {
        module: "pango",
        min_version: Some("1.50"),
    },
];

impl Toolchain for Gtk4Toolchain {
    type Installation = LinuxSystemPackagesInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if !cfg!(target_os = "linux") {
            return Err(ToolchainError::Unfixable(UnfixableToolchain::new(
                "GTK4 backend is only supported on Linux",
                "Run GTK4 targets on Linux with `--platform linux --backend gtk4`.",
            )));
        }

        if !check_pkg_config_exists().await {
            let linux_toolchain = LinuxSystemToolchain;
            return match linux_toolchain.check().await {
                Ok(()) => Err(ToolchainError::Unfixable(UnfixableToolchain::new(
                    "pkg-config not found",
                    "Install pkg-config and ensure it is in PATH, then re-run `water doctor`.",
                ))),
                Err(ToolchainError::Fixable(installation)) => {
                    Err(ToolchainError::Fixable(installation))
                }
                Err(ToolchainError::Unfixable(e)) => Err(ToolchainError::Unfixable(e)),
            };
        }

        let missing = missing_pkg_config_probes().await;
        if missing.is_empty() {
            return Ok(());
        }

        let linux_toolchain = LinuxSystemToolchain;
        return match linux_toolchain.check().await {
            Err(ToolchainError::Fixable(installation)) => {
                Err(ToolchainError::Fixable(installation))
            }
            Err(ToolchainError::Unfixable(e)) => Err(ToolchainError::Unfixable(e)),
            Ok(()) => match gtk4_pkg_config_repair_installation(&missing).await {
                Ok(installation) => Err(ToolchainError::Fixable(installation)),
                Err(error) => {
                    let missing = missing.join(", ");
                    let base_hint = install_gtk4_suggestion();
                    Err(ToolchainError::Unfixable(UnfixableToolchain::new(
                        format!("GTK4 pkg-config probe failed: missing {missing}"),
                        format!(
                            "{base_hint} Also ensure these probes pass: `pkg-config --exists gtk4` and `pkg-config --exists pango && pkg-config --atleast-version=1.50 pango`. Repair planner error: {}",
                            error.message()
                        ),
                    )))
                }
            },
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

async fn check_module_exists(module: &str) -> bool {
    Command::new("pkg-config")
        .args(["--exists", module])
        .status()
        .await
        .is_ok_and(|s| s.success())
}

async fn check_module_min_version(module: &str, min_version: &str) -> bool {
    Command::new("pkg-config")
        .arg(format!("--atleast-version={min_version}"))
        .arg(module)
        .status()
        .await
        .is_ok_and(|s| s.success())
}

async fn missing_pkg_config_probes() -> Vec<String> {
    let mut missing = Vec::new();

    for probe in REQUIRED_PROBES {
        if !check_module_exists(probe.module).await {
            missing.push(probe.display());
            continue;
        }
        if let Some(min_version) = probe.min_version {
            if !check_module_min_version(probe.module, min_version).await {
                missing.push(probe.display());
            }
        }
    }

    missing
}

/// Get platform-specific suggestion for installing GTK4.
fn install_gtk4_suggestion() -> &'static str {
    "GTK4 was not discoverable via pkg-config. Ensure GTK4 development packages are installed for your distribution and `pkg-config --exists gtk4` succeeds."
}

#[cfg(test)]
mod tests {
    use super::{PkgConfigProbe, install_gtk4_suggestion};

    #[test]
    fn gtk4_suggestion_mentions_pkg_config_probe() {
        let gtk4 = install_gtk4_suggestion();
        assert!(gtk4.contains("pkg-config"));
        assert!(gtk4.contains("gtk4"));
    }

    #[test]
    fn pkg_config_probe_display_formats_min_version() {
        let probe = PkgConfigProbe {
            module: "pango",
            min_version: Some("1.50"),
        };
        assert_eq!(probe.display(), "pango>=1.50");
    }

    #[test]
    fn pkg_config_probe_display_without_version() {
        let probe = PkgConfigProbe {
            module: "gtk4",
            min_version: None,
        };
        assert_eq!(probe.display(), "gtk4");
    }
}
