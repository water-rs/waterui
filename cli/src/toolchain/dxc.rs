//! Toolchain support for `dxc`, the DirectX Shader Compiler.
//!
//! It compiles WGSL to DXIL for Windows targets: `shaderloom` and
//! `cherenkov-gpu`'s build script invoke it by name, on a Windows host and on
//! a Linux host cross-building for Windows, so it must sit on the build's
//! `PATH`.
//!
//! Microsoft publishes a release for Windows and for Linux `x86_64` only. On
//! those hosts a missing `dxc` is fixable with the pinned managed release; on
//! any other host it must be provided on `PATH`.

use std::path::PathBuf;

use crate::toolchain::{
    Host, Installation, Toolchain, ToolchainError,
    managed_tool::{self, ManagedToolError},
};

/// `dxc` on the host's `PATH` or under the managed tool directory.
#[derive(Debug, Clone, Copy, Default)]
#[expect(
    clippy::exhaustive_structs,
    reason = "a unit toolchain marker with no fields to add"
)]
pub struct Dxc;

impl Dxc {
    /// The `dxc` executable on `host`: `PATH` first, then the managed install
    /// under `~/.water/tools`.
    pub async fn path(&self, host: &Host) -> Option<PathBuf> {
        if let Ok(path) = host.which("dxc").await {
            return Some(path);
        }
        managed_tool::dxc()?.binary_path(host)
    }
}

impl Toolchain for Dxc {
    type Installation = DxcInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if self.path(host).await.is_some() {
            Ok(())
        } else if managed_tool::dxc().is_some() {
            Err(ToolchainError::fixable(DxcInstallation))
        } else {
            Err(ToolchainError::unfixable(
                "dxc is missing and Microsoft publishes no DirectX Shader Compiler build for this OS and architecture",
                "Install `dxc` from the DirectX Shader Compiler and ensure it is available in PATH.",
            ))
        }
    }
}

/// Install `dxc` from the pinned `microsoft/DirectXShaderCompiler` release
/// into `~/.water/tools`.
#[derive(Debug, Clone, Copy)]
#[expect(
    clippy::exhaustive_structs,
    reason = "a unit toolchain marker with no fields to add"
)]
pub struct DxcInstallation;

/// Errors that can occur while installing `dxc`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FailToInstallDxc {
    /// Microsoft publishes no DirectX Shader Compiler build for this host.
    #[error("Microsoft publishes no DirectX Shader Compiler build for this OS and architecture")]
    NoRelease,
    /// The managed release could not be installed.
    #[error(transparent)]
    Install(#[from] ManagedToolError),
}

impl Installation for DxcInstallation {
    type Error = FailToInstallDxc;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        managed_tool::dxc()
            .ok_or(FailToInstallDxc::NoRelease)?
            .install(host)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Dxc;
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

    #[cfg(any(windows, all(target_os = "linux", target_arch = "x86_64")))]
    #[test]
    fn missing_reports_fixable() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Dxc.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "a host without dxc must report fixable: {result:?}"
        );
    }

    #[test]
    fn ok_when_dxc_on_path() {
        let machine = TestMachine::new();
        machine.install("dxc");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Dxc.check(&host)).expect("dxc on PATH must be ok");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn missing_without_a_managed_release_is_unfixable() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Dxc.check(&host));
        let Err(ToolchainError::Unfixable(error)) = result else {
            panic!("a host without a managed dxc release must report unfixable: {result:?}");
        };
        assert!(error.message().contains("dxc"), "{}", error.message());
        assert!(
            error.suggestion().contains("PATH"),
            "{}",
            error.suggestion()
        );
    }

    #[cfg(any(windows, all(target_os = "linux", target_arch = "x86_64")))]
    #[test]
    fn ok_when_dxc_is_managed() {
        let machine = TestMachine::new();
        let dxc =
            crate::toolchain::managed_tool::dxc().expect("this host has a managed dxc release");
        let install_dir = dxc
            .install_dir(&machine.host(Vec::<(String, String)>::new()))
            .unwrap();
        let binary = install_dir.join(&dxc.binary);
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"").unwrap();

        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Dxc.check(&host)).expect("a managed dxc must satisfy the check");
    }

    #[cfg(any(windows, all(target_os = "linux", target_arch = "x86_64")))]
    #[test]
    fn install_reuses_an_unpacked_copy() {
        let machine = TestMachine::new();
        let dxc =
            crate::toolchain::managed_tool::dxc().expect("this host has a managed dxc release");
        let host = machine.host(Vec::<(String, String)>::new());
        let install_dir = dxc.install_dir(&host).unwrap();
        let binary = install_dir.join(&dxc.binary);
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"").unwrap();

        // With the binary already unpacked the install returns its directory
        // without touching the network — the pre-staged file is the proof,
        // since a download would overwrite it with archive contents.
        let dir = smol::block_on(dxc.install(&host)).expect("install must succeed");
        assert_eq!(dir, binary.parent().unwrap());
        assert_eq!(std::fs::read(&binary).unwrap(), b"");
    }
}
