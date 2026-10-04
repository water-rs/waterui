//! Cargo-installed helper binaries the CLI's workflows invoke.
//!
//! These are binaries cargo puts on `PATH` (an entry under
//! `$CARGO_HOME/bin`), not rustup components: `cargo-nextest` for
//! `water bench`, `espflash`/`ldproxy` for the ESP32 backend. The check probes
//! `PATH` and the repair is the `cargo install`/`cargo binstall` command that
//! produces the binary.

use crate::{
    toolchain::{Host, Installation, Toolchain, ToolchainError},
    utils::CommandError,
};

/// A required set of cargo-installed helper binaries on `PATH`.
#[derive(Debug, Clone)]
pub struct CargoHelpers {
    required: Vec<String>,
}

impl CargoHelpers {
    /// Check that each of `required` binaries resolves on `PATH`.
    #[must_use]
    pub fn new(required: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            required: required.into_iter().map(Into::into).collect(),
        }
    }
}

/// Installation plan installing missing helper crates with cargo.
#[derive(Debug, Clone)]
pub struct CargoHelpersInstallation {
    crates: Vec<String>,
}

impl CargoHelpersInstallation {
    /// Plan `cargo install` (or `cargo binstall` when available) for `crates`.
    pub(crate) const fn new(crates: Vec<String>) -> Self {
        Self { crates }
    }

    /// The missing helpers and their install command, for the doctor item's
    /// message.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "cargo helpers not on PATH: {} (installed with `cargo install {}`)",
            self.crates.join(", "),
            self.crates.join(" ")
        )
    }
}

/// Errors from `cargo install`/`cargo binstall` helper installation.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallCargoHelpers {
    /// cargo was not found.
    #[error(
        "cargo is required to install cargo helpers but is not on PATH; fix the `rust` doctor item first."
    )]
    CargoNotFound,
    /// A helper crate could not be installed.
    #[error("Failed to install `{krate}` with cargo: {source}")]
    Install {
        /// Crate that failed to install.
        krate: String,
        /// Underlying command error.
        source: CommandError,
    },
}

impl Toolchain for CargoHelpers {
    type Installation = CargoHelpersInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        let mut missing = Vec::new();
        for binary in &self.required {
            if host.which(binary).await.is_err() {
                missing.push(binary.clone());
            }
        }
        if missing.is_empty() {
            return Ok(());
        }
        if host.which("cargo").await.is_err() {
            return Err(ToolchainError::unfixable(
                format!("cargo helpers not on PATH: {}", missing.join(", ")),
                format!(
                    "Install Rust via rustup (fix the `rust` doctor item first), then run `cargo install {}`.",
                    missing.join(" ")
                ),
            ));
        }
        Err(ToolchainError::fixable(CargoHelpersInstallation::new(
            missing,
        )))
    }
}

impl Installation for CargoHelpersInstallation {
    type Error = FailToInstallCargoHelpers;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if host.which("cargo").await.is_err() {
            return Err(FailToInstallCargoHelpers::CargoNotFound);
        }
        // `cargo binstall` fetches a prebuilt binary instead of compiling
        // from source; use it when the host carries it.
        let binstall = host.which("cargo-binstall").await.is_ok();
        for krate in &self.crates {
            if binstall {
                host.run("cargo", ["binstall", "--no-confirm", krate.as_str()])
                    .await
            } else {
                host.run("cargo", ["install", "--locked", krate.as_str()])
                    .await
            }
            .map_err(|source| FailToInstallCargoHelpers::Install {
                krate: krate.clone(),
                source,
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::CargoHelpers;
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

    #[test]
    fn helpers_ok_when_on_path() {
        let machine = TestMachine::new();
        machine.install("cargo-nextest");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(CargoHelpers::new(["cargo-nextest"]).check(&host))
            .expect("helper on PATH must be ok");
    }

    #[test]
    fn helpers_fixable_when_cargo_present() {
        let machine = TestMachine::new();
        machine.install("cargo");
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(CargoHelpers::new(["cargo-nextest"]).check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing helper with cargo present must be fixable: {result:?}"
        );
    }

    #[test]
    fn helpers_unfixable_without_cargo() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(CargoHelpers::new(["cargo-nextest"]).check(&host));
        match &result {
            Err(ToolchainError::Unfixable(error)) => {
                assert!(
                    error.suggestion().contains("cargo install"),
                    "the manual repair must name cargo install: {}",
                    error.suggestion()
                );
            }
            other => panic!("missing helper without cargo must be manual: {other:?}"),
        }
    }
}
