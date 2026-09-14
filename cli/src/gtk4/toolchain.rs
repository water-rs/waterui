//! GTK4 toolchain checking.

use crate::toolchain::{
    Host, Toolchain, ToolchainError, UnfixableToolchain,
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
        self.min_version.map_or_else(
            || self.module.to_owned(),
            |min| format!("{}>={min}", self.module),
        )
    }
}

const REQUIRED_PROBES: &[PkgConfigProbe] = &[
    // The same floor as the `v4_14` feature in `backends/gtk/Cargo.toml`:
    // clipping to an arbitrary path is `gtk_snapshot_push_fill`, which is 4.14.
    PkgConfigProbe {
        module: "gtk4",
        min_version: Some("4.14"),
    },
    PkgConfigProbe {
        module: "pango",
        min_version: Some("1.50"),
    },
];

impl Toolchain for Gtk4Toolchain {
    type Installation = LinuxSystemPackagesInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if !cfg!(target_os = "linux") {
            return Err(ToolchainError::Unfixable(UnfixableToolchain::new(
                "GTK4 backend is only supported on Linux",
                "Run GTK4 targets on Linux with `--platform linux --backend gtk4`.",
            )));
        }

        if !check_pkg_config_exists(host).await {
            let linux_toolchain = LinuxSystemToolchain;
            return match linux_toolchain.check(host).await {
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

        let missing = missing_pkg_config_probes(host).await;
        if missing.is_empty() {
            return Ok(());
        }

        let linux_toolchain = LinuxSystemToolchain;
        return match linux_toolchain.check(host).await {
            Err(ToolchainError::Fixable(installation)) => {
                Err(ToolchainError::Fixable(installation))
            }
            Err(ToolchainError::Unfixable(e)) => Err(ToolchainError::Unfixable(e)),
            Ok(()) => match gtk4_pkg_config_repair_installation(host, &missing).await {
                Ok(installation) => Err(ToolchainError::Fixable(installation)),
                Err(error) => {
                    let missing = missing.join(", ");
                    let base_hint = install_gtk4_suggestion();
                    Err(ToolchainError::Unfixable(UnfixableToolchain::new(
                        format!("GTK4 pkg-config probe failed: missing {missing}"),
                        format!(
                            "{base_hint} Also ensure these probes pass: `pkg-config --exists gtk4 && pkg-config --atleast-version=4.14 gtk4` and `pkg-config --exists pango && pkg-config --atleast-version=1.50 pango`. Repair planner error: {}",
                            error.message()
                        ),
                    )))
                }
            },
        };
    }
}

/// Check if pkg-config is available.
async fn check_pkg_config_exists(host: &Host) -> bool {
    host.output("pkg-config", ["--version"])
        .await
        .is_ok_and(|o| o.status.success())
}

async fn check_module_exists(host: &Host, module: &str) -> bool {
    host.output("pkg-config", ["--exists", module])
        .await
        .is_ok_and(|o| o.status.success())
}

async fn check_module_min_version(host: &Host, module: &str, min_version: &str) -> bool {
    host.output(
        "pkg-config",
        [
            format!("--atleast-version={min_version}"),
            module.to_owned(),
        ],
    )
    .await
    .is_ok_and(|o| o.status.success())
}

async fn missing_pkg_config_probes(host: &Host) -> Vec<String> {
    let mut missing = Vec::new();

    for probe in REQUIRED_PROBES {
        if !check_module_exists(host, probe.module).await {
            missing.push(probe.display());
            continue;
        }
        if let Some(min_version) = probe.min_version
            && !check_module_min_version(host, probe.module, min_version).await
        {
            missing.push(probe.display());
        }
    }

    missing
}

/// Get platform-specific suggestion for installing GTK4.
const fn install_gtk4_suggestion() -> &'static str {
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

#[cfg(test)]
mod host_tests {
    use super::Gtk4Toolchain;
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn off_linux_is_unfixable() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Gtk4Toolchain.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "GTK4 outside Linux must be unfixable: {result:?}"
        );
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use super::*;

        const APT_PACKAGES: &str = "pkg-config libgtk-4-dev libpango1.0-dev libwayland-dev \
             wayland-protocols libasound2-dev libva-dev libgbm-dev libxcb1-dev \
             libclang-dev libfontconfig-dev";

        /// Machine with pkg-config and both GTK probes satisfied, and an apt
        /// package set that leaves `LinuxSystemToolchain` satisfied as well.
        fn complete_machine() -> TestMachine {
            let machine = TestMachine::new();
            for tool in ["apt-get", "dpkg-query", "pkg-config"] {
                machine.install(tool);
            }
            machine.respond_pkg_config_module("gtk4", "4.18.0");
            machine.respond_pkg_config_module("pango", "1.56.0");
            machine.respond_pkg_config_module("libva", "1.20.0");
            machine.respond_pkg_config_var("libva_version", "2.20.0");
            machine.respond_pkg_config_module("libpipewire-0.3", "0.3.65");
            machine
        }

        #[test]
        fn ok_when_probes_and_packages_satisfied() {
            let machine = complete_machine();
            let host = machine.host([(
                String::from("WATERUI_FAKE_DPKG_INSTALLED"),
                APT_PACKAGES.to_string(),
            )]);
            smol::block_on(Gtk4Toolchain.check(&host))
                .expect("satisfied gtk4/pango probes plus complete apt set must be ok");
        }

        #[test]
        fn missing_probe_with_apt_is_fixable() {
            let machine = TestMachine::new();
            for tool in ["apt-get", "dpkg-query", "pkg-config"] {
                machine.install(tool);
            }
            let host = machine.host([(
                String::from("WATERUI_FAKE_DPKG_INSTALLED"),
                String::from("pkg-config"),
            )]);
            let result = smol::block_on(Gtk4Toolchain.check(&host));
            assert!(
                matches!(result, Err(ToolchainError::Fixable(_))),
                "missing gtk4 probes under apt must produce a fixable install: {result:?}"
            );
        }

        #[test]
        fn missing_probes_fixable_via_repair_when_packages_complete() {
            let machine = complete_machine();
            // Drop the gtk4 module response: `--exists gtk4` now fails while
            // every apt package still reports installed, so the fix must come
            // from the repair-installation path. `PKG_CONFIG_GTK4` is the
            // response key the dispatcher derives for `gtk4` — the raw name.
            std::fs::remove_file(machine.responses().join("PKG_CONFIG_gtk4"))
                .expect("remove staged gtk4 module response");
            let host = machine.host([(
                String::from("WATERUI_FAKE_DPKG_INSTALLED"),
                APT_PACKAGES.to_string(),
            )]);
            let result = smol::block_on(Gtk4Toolchain.check(&host));
            assert!(
                matches!(result, Err(ToolchainError::Fixable(_))),
                "complete packages + missing gtk4 probe must repair via package mapping: {result:?}"
            );
        }

        #[test]
        fn unfixable_without_package_manager() {
            let machine = TestMachine::new();
            machine.install("pkg-config");
            let host = machine.host(Vec::<(String, String)>::new());
            let result = smol::block_on(Gtk4Toolchain.check(&host));
            assert!(
                matches!(result, Err(ToolchainError::Unfixable(_))),
                "no package manager must be unfixable: {result:?}"
            );
        }

        #[test]
        fn unfixable_when_pkg_config_missing_but_packages_installed() {
            let machine = TestMachine::new();
            machine.install("apt-get");
            machine.install("dpkg-query");
            let host = machine.host([(
                String::from("WATERUI_FAKE_DPKG_INSTALLED"),
                APT_PACKAGES.to_string(),
            )]);
            let result = smol::block_on(Gtk4Toolchain.check(&host));
            assert!(
                matches!(result, Err(ToolchainError::Unfixable(_))),
                "installed packages without pkg-config must be unfixable: {result:?}"
            );
        }
    }
}
