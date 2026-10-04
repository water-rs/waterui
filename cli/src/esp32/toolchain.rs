//! ESP32 (Dew) toolchain checks and remediation.
//!
//! An ESP32 build drives a non-rustup toolchain — the Espressif `esp` Rust
//! fork under `~/.rustup/toolchains/esp` — plus the pieces that live beside
//! it: the Espressif clang libraries (`LIBCLANG_PATH` for `esp-idf-sys`'s
//! bindgen), the chip architecture's GCC linker toolchain, `rust-src` (the
//! generated harness builds `std` itself via `-Zbuild-std`), and the
//! cargo-installed `espflash`/`ldproxy` binaries the build and run paths
//! invoke. `espup install` is the programmatic repair for everything under
//! the toolchain directories; QEMU, needed for emulated `water run`, comes
//! from the system package manager.

use std::path::{Path, PathBuf};

use crate::{
    esp32::{
        chip::{Esp32Arch, Esp32Chip},
        platform::newest_toolchain_subpath,
    },
    toolchain::{
        Host, Installation, Toolchain, ToolchainError,
        cargo_helpers::{CargoHelpersInstallation, FailToInstallCargoHelpers},
        rust::rustup_toolchains_dir,
    },
    utils::CommandError,
};

/// ESP32 toolchain checker for a set of chips.
///
/// The chip set is what `[esp32]` selects — or every supported chip,
/// since a project can target any of them. Pieces shared across
/// chips (the `esp` toolchain, its clang libraries, `rust-src`, the helper
/// binaries) are probed once; the architecture-specific GCC and QEMU binary
/// once per architecture.
#[derive(Debug, Clone)]
pub struct Esp32Toolchain {
    chips: Vec<Esp32Chip>,
}

impl Esp32Toolchain {
    /// Check the ESP32 toolchain for `chips`.
    #[must_use]
    pub fn new(chips: impl IntoIterator<Item = Esp32Chip>) -> Self {
        Self {
            chips: chips.into_iter().collect(),
        }
    }
}

/// Installation plan for the ESP32 toolchain.
///
/// `cargo install espup` runs when the installer itself is absent, `espup
/// install` repairs the toolchain directories (`--esp-riscv-gcc` when the
/// chip's GCC is RISC-V and missing), then `cargo install` covers the
/// helper binaries.
#[derive(Debug, Clone)]
pub struct Esp32ToolchainInstallation {
    /// What the check found missing — surfaced as the doctor item's message.
    missing: Vec<String>,
    /// Pieces no automatic repair covers (e.g. QEMU).
    manual: Vec<String>,
    /// Run `cargo install espup` before `espup install`.
    install_espup: bool,
    /// Run `espup install` (with `--esp-riscv-gcc` when `riscv_gcc`).
    run_espup: bool,
    /// A RISC-V chip's GCC was missing — pass `--esp-riscv-gcc`.
    riscv_gcc: bool,
    /// `cargo install`/`cargo binstall` for missing helper binaries.
    helpers: CargoHelpersInstallation,
}

impl Esp32ToolchainInstallation {
    /// The missing pieces and manual repairs, for the doctor item's message.
    #[must_use]
    pub fn describe(&self) -> String {
        let manual = if self.manual.is_empty() {
            String::new()
        } else {
            format!(". Manual steps: {}", self.manual.join("; "))
        };
        format!(
            "ESP32 toolchain incomplete: {}{manual}",
            self.missing.join(", ")
        )
    }
}

/// Errors from ESP32 toolchain installation.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallEsp32Toolchain {
    /// `cargo install espup` failed.
    #[error("Failed to install `espup`: {0}")]
    InstallEspup(#[source] FailToInstallCargoHelpers),
    /// `espup install` failed.
    #[error("`espup install` failed: {0}")]
    EspupInstall(#[source] CommandError),
    /// A helper binary could not be installed.
    #[error(transparent)]
    Helpers(#[from] FailToInstallCargoHelpers),
}

/// The Espressif `esp` toolchain root on this host:
/// `$RUSTUP_HOME/toolchains/esp`, or `~/.rustup/toolchains/esp`.
fn esp_toolchain_dir(host: &Host) -> Option<PathBuf> {
    rustup_toolchains_dir(host).map(|dir| dir.join("esp"))
}

/// The install hint for QEMU's system emulator on this OS.
const fn qemu_install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "brew install qemu"
    } else if cfg!(target_os = "windows") {
        "install QEMU from https://www.qemu.org/download/#windows or `winget install QEMU`"
    } else {
        "install the qemu-system package (e.g. `apt install qemu-system-misc`)"
    }
}

/// The gaps the ESP32 probes accumulate before classification.
#[derive(Default)]
struct Esp32Findings {
    /// What is missing — surfaced verbatim in the doctor item's message.
    missing: Vec<String>,
    /// Repairs no automatic step covers (QEMU, an espup gap it cannot fill).
    manual: Vec<String>,
    /// `espup install` repairs the toolchain directories.
    run_espup: bool,
    /// The RISC-V GCC was missing — pass `--esp-riscv-gcc`.
    riscv_gcc: bool,
    /// Cargo-installable helper binaries missing from PATH.
    helpers: Vec<String>,
}

impl Esp32Toolchain {
    /// The `esp` toolchain directory and the pieces living inside it: the
    /// Espressif clang libraries `esp-idf-sys`'s bindgen needs, `rust-src`
    /// (the generated harness builds `std` via `-Zbuild-std`), and the
    /// Xtensa GCC the toolchain ships.
    fn probe_esp_toolchain(&self, host: &Host, findings: &mut Esp32Findings) {
        let Some(esp_dir) = esp_toolchain_dir(host).filter(|dir| dir.is_dir()) else {
            findings.missing.push("the `esp` Rust toolchain".to_owned());
            findings.run_espup = true;
            return;
        };
        if newest_toolchain_subpath(
            &esp_dir.join("xtensa-esp32-elf-clang"),
            Path::new("esp-clang/lib"),
        )
        .is_none()
        {
            findings
                .missing
                .push("the Espressif clang libraries".to_owned());
            findings.run_espup = true;
        }
        if !esp_dir.join("lib/rustlib/src/rust").is_dir() {
            findings
                .missing
                .push("the `rust-src` component on the `esp` toolchain".to_owned());
            findings.run_espup = true;
        }
        if self
            .chips
            .iter()
            .any(|chip| chip.arch() == Esp32Arch::Xtensa)
        {
            let gcc = Esp32Chip::Esp32S3.gcc_component();
            if newest_toolchain_subpath(&esp_dir.join(gcc.component), Path::new(gcc.bin_subpath))
                .is_none()
            {
                findings
                    .missing
                    .push(format!("the {} ({})", gcc.what, gcc.component));
                findings.run_espup = true;
            }
        }
    }

    /// The RISC-V GCC lives outside the `esp` toolchain, under
    /// `~/.espressif/tools`, so it is probed even when `esp` is missing.
    fn probe_riscv_gcc(&self, host: &Host, findings: &mut Esp32Findings) {
        let Some(chip) = self
            .chips
            .iter()
            .find(|chip| chip.arch() == Esp32Arch::RiscV)
        else {
            return;
        };
        let gcc = chip.gcc_component();
        let base = host
            .home_dir()
            .map(|home| home.join(".espressif/tools").join(gcc.component));
        let present = base.as_ref().is_some_and(|base| {
            newest_toolchain_subpath(base, Path::new(gcc.bin_subpath)).is_some()
        });
        if present {
            return;
        }
        findings.missing.push(format!(
            "the {} (`{}` under ~/.espressif/tools)",
            gcc.what, gcc.component
        ));
        // `espup install --esp-riscv-gcc` installs Espressif's RISC-V GCC;
        // ESP-IDF's `idf_tools.py install` is the alternative.
        findings.run_espup = true;
        findings.riscv_gcc = true;
        findings.manual.push(format!(
            "if `espup install --esp-riscv-gcc` does not provide the {}, install it with ESP-IDF's `idf_tools.py install`",
            gcc.what
        ));
    }

    /// `water run`/`water package` for ESP32 invoke `espflash` directly and
    /// the generated harness links with `ldproxy`; emulated `water run`
    /// boots the chip under QEMU's system emulator.
    async fn probe_binaries(&self, host: &Host, findings: &mut Esp32Findings) {
        for binary in ["espflash", "ldproxy"] {
            if host.which(binary).await.is_err() {
                findings.missing.push(format!("`{binary}` on PATH"));
                findings.helpers.push(binary.to_owned());
            }
        }
        let mut qemu_checked = Vec::<&'static str>::new();
        for qemu in self.chips.iter().map(|chip| chip.qemu_binary()) {
            if qemu_checked.contains(&qemu) {
                continue;
            }
            qemu_checked.push(qemu);
            if host.which(qemu).await.is_err() {
                findings
                    .missing
                    .push(format!("`{qemu}` on PATH (for emulated `water run`)"));
                findings
                    .manual
                    .push(format!("install QEMU with `{}`", qemu_install_hint()));
            }
        }
    }
}

impl Toolchain for Esp32Toolchain {
    type Installation = Esp32ToolchainInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        let mut findings = Esp32Findings::default();
        self.probe_esp_toolchain(host, &mut findings);
        self.probe_riscv_gcc(host, &mut findings);
        self.probe_binaries(host, &mut findings).await;

        if findings.missing.is_empty() {
            return Ok(());
        }

        let cargo_available = host.which("cargo").await.is_ok();
        let espup_available = host.which("espup").await.is_ok();
        let helpers_missing = !findings.helpers.is_empty();
        let install_espup = findings.run_espup && !espup_available;
        // `espup` itself, and the cargo helpers, are cargo installs — nothing
        // is programmatically repairable without cargo on PATH.
        if !cargo_available && (install_espup || helpers_missing) {
            let mut commands = Vec::new();
            if install_espup {
                commands.push("cargo install espup".to_owned());
            }
            if findings.run_espup {
                commands.push("espup install".to_owned());
            }
            if helpers_missing {
                commands.push(format!("cargo install {}", findings.helpers.join(" ")));
            }
            return Err(ToolchainError::unfixable(
                format!(
                    "ESP32 toolchain incomplete: {}",
                    findings.missing.join(", ")
                ),
                format!(
                    "Install Rust via rustup first (see the `rust` doctor item), then run {}.",
                    commands.join("`, `")
                ),
            ));
        }

        let installation = Esp32ToolchainInstallation {
            missing: findings.missing,
            manual: findings.manual,
            install_espup,
            run_espup: findings.run_espup,
            riscv_gcc: findings.riscv_gcc,
            helpers: CargoHelpersInstallation::new(findings.helpers),
        };
        if install_espup || findings.run_espup || helpers_missing {
            Err(ToolchainError::fixable(installation))
        } else {
            // Only manual pieces are missing (e.g. QEMU alone).
            let manual = installation.manual.join("; ");
            Err(ToolchainError::unfixable(installation.describe(), manual))
        }
    }
}

impl Installation for Esp32ToolchainInstallation {
    type Error = FailToInstallEsp32Toolchain;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if self.install_espup {
            CargoHelpersInstallation::new(vec!["espup".to_owned()])
                .install(host)
                .await
                .map_err(FailToInstallEsp32Toolchain::InstallEspup)?;
        }
        if self.run_espup {
            let mut args = vec!["install"];
            if self.riscv_gcc {
                args.push("--esp-riscv-gcc");
            }
            host.run("espup", args)
                .await
                .map_err(FailToInstallEsp32Toolchain::EspupInstall)?;
        }
        self.helpers.install(host).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Esp32Toolchain;
    use crate::esp32::chip::Esp32Chip;
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

    /// Stage the shared `esp` toolchain directories (`clang` libs, `rust-src`)
    /// and, optionally, the Xtensa GCC under the fake home.
    fn stage_esp_toolchain(machine: &TestMachine, xtensa_gcc: bool) {
        for subdir in [
            "xtensa-esp32-elf-clang/1.0/esp-clang/lib",
            "lib/rustlib/src/rust",
        ] {
            machine.dir(format!("home/.rustup/toolchains/esp/{subdir}"));
        }
        if xtensa_gcc {
            machine.dir("home/.rustup/toolchains/esp/xtensa-esp-elf/1.0/xtensa-esp-elf/bin");
        }
    }

    /// A host with cargo and the `espup`/`espflash`/`ldproxy`/QEMU binaries
    /// but no `esp` toolchain — everything missing is a cargo or espup
    /// install away.
    fn cargo_machine() -> TestMachine {
        let machine = TestMachine::new();
        machine.install("cargo");
        machine
    }

    #[test]
    fn esp32_unfixable_when_esp_missing_and_no_cargo() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Esp32Toolchain::new([Esp32Chip::Esp32S3]).check(&host));
        match &result {
            Err(ToolchainError::Unfixable(error)) => {
                assert!(
                    error.suggestion().contains("cargo install espup"),
                    "the manual path must name `cargo install espup`: {}",
                    error.suggestion()
                );
            }
            other => panic!("missing esp toolchain without cargo must be manual: {other:?}"),
        }
    }

    #[test]
    fn esp32_fixable_when_esp_missing_and_cargo_present() {
        let machine = cargo_machine();
        machine.install("espup");
        machine.install("espflash");
        machine.install("ldproxy");
        machine.install("qemu-system-xtensa");
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Esp32Toolchain::new([Esp32Chip::Esp32S3]).check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing esp toolchain with espup present must be fixable: {result:?}"
        );
    }

    #[test]
    fn esp32_ok_when_fully_staged() {
        let machine = cargo_machine();
        machine.install("espflash");
        machine.install("ldproxy");
        machine.install("qemu-system-xtensa");
        stage_esp_toolchain(&machine, true);
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Esp32Toolchain::new([Esp32Chip::Esp32S3]).check(&host))
            .expect("a fully staged esp toolchain must be ok");
    }

    #[test]
    fn esp32_riscv_checks_espressif_tools_tree() {
        let machine = cargo_machine();
        machine.install("espflash");
        machine.install("ldproxy");
        machine.install("qemu-system-riscv32");
        stage_esp_toolchain(&machine, false);
        // The RISC-V GCC is still missing under ~/.espressif/tools.
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Esp32Toolchain::new([Esp32Chip::Esp32C3]).check(&host));
        match &result {
            Err(ToolchainError::Fixable(installation)) => {
                assert!(
                    installation.describe().contains("riscv32-esp-elf"),
                    "the missing RISC-V GCC must be named: {}",
                    installation.describe()
                );
            }
            other => panic!("a missing RISC-V GCC must be fixable via espup: {other:?}"),
        }
    }

    #[test]
    fn esp32_riscv_ok_when_gcc_staged() {
        let machine = cargo_machine();
        machine.install("espflash");
        machine.install("ldproxy");
        machine.install("qemu-system-riscv32");
        stage_esp_toolchain(&machine, false);
        machine.dir("home/.espressif/tools/riscv32-esp-elf/1.0/riscv32-esp-elf/bin");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Esp32Toolchain::new([Esp32Chip::Esp32C3]).check(&host))
            .expect("staged RISC-V toolchain must be ok");
    }

    #[test]
    fn esp32_qemu_alone_is_manual() {
        let machine = cargo_machine();
        machine.install("espflash");
        machine.install("ldproxy");
        stage_esp_toolchain(&machine, true);
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Esp32Toolchain::new([Esp32Chip::Esp32S3]).check(&host));
        match &result {
            Err(ToolchainError::Unfixable(error)) => {
                assert!(
                    error.suggestion().contains("QEMU"),
                    "the QEMU-only gap must name its install: {}",
                    error.suggestion()
                );
            }
            other => panic!("only QEMU missing must be a manual item: {other:?}"),
        }
    }
}
