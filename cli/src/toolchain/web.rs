//! Web toolchain checks and installations.

use color_eyre::eyre;

use crate::{
    toolchain::{Installation, Toolchain, ToolchainError},
    utils::{run_command, run_command_output_os, which},
};

/// Rust `wasm32-unknown-unknown` target support.
#[derive(Debug, Clone, Copy, Default)]
pub struct Wasm32UnknownUnknownTarget;

/// Installation plan for the Rust wasm target.
#[derive(Debug, Clone, Copy, Default)]
pub struct Wasm32UnknownUnknownTargetInstallation;

impl Toolchain for Wasm32UnknownUnknownTarget {
    type Installation = Wasm32UnknownUnknownTargetInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if which("rustup").await.is_err() {
            return Err(ToolchainError::unfixable(
                "rustup is not installed or not found in PATH",
                "Install Rust with rustup from https://rustup.rs/ and re-run the command.",
            ));
        }

        let output = run_command_output_os("rustup", ["target", "list", "--installed"]).await;
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
    type Error = eyre::Report;

    async fn install(&self) -> Result<(), Self::Error> {
        run_command("rustup", ["target", "add", "wasm32-unknown-unknown"])
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

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if which("wasm-pack").await.is_ok() {
            Ok(())
        } else {
            Err(ToolchainError::Fixable(WasmPackInstallation))
        }
    }
}

impl Installation for WasmPackInstallation {
    type Error = eyre::Report;

    async fn install(&self) -> Result<(), Self::Error> {
        run_command("cargo", ["install", "wasm-pack"])
            .await
            .map(|_| ())
    }
}

/// Composite toolchain for Web/WASM support.
pub type WebToolchain = (Wasm32UnknownUnknownTarget, WasmPack);
