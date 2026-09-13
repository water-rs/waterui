//! Web toolchain checks and installations.

use crate::{
    toolchain::{Host, Installation, Toolchain, ToolchainError},
    utils::CommandError,
    web::PackageManager,
};

/// Rust `wasm32-unknown-unknown` target support.
#[derive(Debug, Clone, Copy, Default)]
pub struct Wasm32UnknownUnknownTarget;

/// Installation plan for the Rust wasm target.
#[derive(Debug, Clone, Copy, Default)]
pub struct Wasm32UnknownUnknownTargetInstallation;

impl Toolchain for Wasm32UnknownUnknownTarget {
    type Installation = Wasm32UnknownUnknownTargetInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if host.which("rustup").await.is_err() {
            return Err(ToolchainError::unfixable(
                "rustup is not installed or not found in PATH",
                "Install Rust with rustup from https://rustup.rs/ and re-run the command.",
            ));
        }

        let output = host
            .output("rustup", ["target", "list", "--installed"])
            .await;
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                return Err(ToolchainError::unfixable(
                    format!("failed to query installed rustup targets: {error}"),
                    "Ensure rustup is healthy, then run `rustup target add wasm32-unknown-unknown` manually.",
                ));
            }
        };

        if !output.status.success() {
            return Err(ToolchainError::unfixable(
                format!(
                    "rustup target query failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
                "Ensure rustup is healthy, then run `rustup target add wasm32-unknown-unknown` manually.",
            ));
        }

        let installed = String::from_utf8_lossy(&output.stdout);
        if installed
            .lines()
            .any(|line| line.trim() == "wasm32-unknown-unknown")
        {
            Ok(())
        } else {
            Err(ToolchainError::Fixable(
                Wasm32UnknownUnknownTargetInstallation,
            ))
        }
    }
}

impl Installation for Wasm32UnknownUnknownTargetInstallation {
    type Error = CommandError;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        host.run("rustup", ["target", "add", "wasm32-unknown-unknown"])
            .await
            .map(|_| ())
    }
}

/// `wasm-pack` binary for packaging browser bundles.
#[derive(Debug, Clone, Copy, Default)]
pub struct WasmPack;

/// Installation plan for `wasm-pack`.
#[derive(Debug, Clone, Copy, Default)]
pub struct WasmPackInstallation;

impl Toolchain for WasmPack {
    type Installation = WasmPackInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if host.which("wasm-pack").await.is_ok() {
            Ok(())
        } else {
            Err(ToolchainError::Fixable(WasmPackInstallation))
        }
    }
}

impl Installation for WasmPackInstallation {
    type Error = CommandError;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        host.run("cargo", ["install", "wasm-pack"])
            .await
            .map(|_| ())
    }
}

/// The `[web] package_manager` a project declares: the executable the CLI
/// invokes for frontend builds and scaffolding. Doctor checks the declared
/// manager only — it never substitutes another.
#[derive(Debug, Clone, Copy)]
pub struct PackageManagerToolchain(pub PackageManager);

/// Installation plan for a declared package manager, using its official
/// installer.
#[derive(Debug, Clone, Copy)]
pub struct PackageManagerInstallation(pub PackageManager);

impl Toolchain for PackageManagerToolchain {
    type Installation = PackageManagerInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        let package_manager = self.0;
        if host.which(package_manager.binary()).await.is_ok() {
            return Ok(());
        }
        match package_manager {
            // npm ships with Node.js; there is no official standalone
            // installer, so this stays a manual step.
            PackageManager::Npm => Err(ToolchainError::unfixable(
                "npm is not installed",
                package_manager.install_hint(),
            )),
            _ => Err(ToolchainError::fixable(PackageManagerInstallation(
                package_manager,
            ))),
        }
    }
}

impl Installation for PackageManagerInstallation {
    type Error = eyre::Report;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        match self.0 {
            // Yarn's official distribution is the corepack shim.
            PackageManager::Yarn => host
                .run("corepack", ["enable"])
                .await
                .map(|_| ())
                .map_err(Into::into),
            PackageManager::Npm => Err(eyre::eyre!(
                "npm ships with Node.js; install Node.js from https://nodejs.org/"
            )),
            package_manager => {
                #[cfg(unix)]
                {
                    let script = match package_manager {
                        PackageManager::Bun => "curl -fsSL https://bun.sh/install | bash",
                        PackageManager::Pnpm => "curl -fsSL https://get.pnpm.io/install.sh | sh -",
                        _ => unreachable!(),
                    };
                    host.run("sh", ["-c", script])
                        .await
                        .map(|_| ())
                        .map_err(Into::into)
                }
                #[cfg(windows)]
                {
                    let script = match package_manager {
                        PackageManager::Bun => "irm bun.sh/install.ps1 | iex",
                        PackageManager::Pnpm => "iwr https://get.pnpm.io/install.ps1 -useb | iex",
                        _ => unreachable!(),
                    };
                    host.run("powershell", ["-c", script])
                        .await
                        .map(|_| ())
                        .map_err(Into::into)
                }
                #[cfg(not(any(unix, windows)))]
                {
                    Err(eyre::eyre!(
                        "no automatic installer for {} on this platform; run: {}",
                        package_manager.binary(),
                        package_manager.install_hint()
                    ))
                }
            }
        }
    }
}

/// Composite toolchain for Web/WASM support.
pub type WebToolchain = (Wasm32UnknownUnknownTarget, WasmPack);

#[cfg(test)]
mod tests {
    use super::{Wasm32UnknownUnknownTarget, WasmPack};
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

    fn machine_with_rustup() -> TestMachine {
        let machine = TestMachine::new();
        machine.install("rustup");
        machine
    }

    #[test]
    fn wasm32_target_unfixable_without_rustup() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Wasm32UnknownUnknownTarget.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "missing rustup must be unfixable: {result:?}"
        );
    }

    #[test]
    fn wasm32_target_fixable_when_not_installed() {
        let machine = machine_with_rustup();
        let host = machine.host([(
            String::from("WATERUI_FAKE_RUSTUP_INSTALLED_TARGETS"),
            String::from("aarch64-apple-darwin"),
        )]);
        let result = smol::block_on(Wasm32UnknownUnknownTarget.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "absent wasm32 target must be fixable: {result:?}"
        );
    }

    #[test]
    fn wasm32_target_ok_when_installed() {
        let machine = machine_with_rustup();
        // `rustup target list --installed` emits one target per line.
        machine.respond(
            "RUSTUP_INSTALLED_TARGETS",
            &["aarch64-apple-darwin", "wasm32-unknown-unknown"].join("\n"),
        );
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Wasm32UnknownUnknownTarget.check(&host))
            .expect("installed wasm32 target must be ok");
    }

    #[test]
    fn wasm_pack_ok_when_on_path() {
        let machine = TestMachine::new();
        machine.install("wasm-pack");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(WasmPack.check(&host)).expect("wasm-pack on PATH must be ok");
    }

    #[test]
    fn wasm_pack_fixable_when_missing() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(WasmPack.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing wasm-pack must be fixable via cargo install: {result:?}"
        );
    }
}
