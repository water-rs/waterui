//! Toolchain support for `dxc`, the DirectX Shader Compiler that compiles
//! Hydrolysis shaders on Windows builds.
//!
//! `shaderloom` invokes it by name from its build script, so it must sit on
//! the build's `PATH`.

use std::path::PathBuf;

use crate::toolchain::{
    Host, Installation, Toolchain, ToolchainError,
    managed_tool::{self, ManagedToolError},
};

/// `dxc` on the host's `PATH` or under the managed tool directory.
#[derive(Debug, Clone, Copy, Default)]
pub struct Dxc;

impl Dxc {
    /// The `dxc` executable on `host`: `PATH` first, then the managed install
    /// under `~/.water/tools`.
    pub async fn path(&self, host: &Host) -> Option<PathBuf> {
        if let Ok(path) = host.which("dxc").await {
            return Some(path);
        }
        managed_tool::dxc().binary_path(host)
    }
}

impl Toolchain for Dxc {
    type Installation = DxcInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if self.path(host).await.is_some() {
            Ok(())
        } else {
            Err(ToolchainError::fixable(DxcInstallation))
        }
    }
}

/// Install `dxc` from the pinned `microsoft/DirectXShaderCompiler` release
/// into `~/.water/tools`.
#[derive(Debug, Clone, Copy)]
pub struct DxcInstallation;

/// Errors that can occur while installing `dxc`.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct FailToInstallDxc(#[from] ManagedToolError);

impl Installation for DxcInstallation {
    type Error = FailToInstallDxc;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        managed_tool::dxc().install(host).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Dxc;
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

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

    #[test]
    fn ok_when_dxc_is_managed() {
        let machine = TestMachine::new();
        let dxc = crate::toolchain::managed_tool::dxc();
        let install_dir = dxc
            .install_dir(&machine.host(Vec::<(String, String)>::new()))
            .unwrap();
        let binary = install_dir.join(&dxc.binary);
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"").unwrap();

        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Dxc.check(&host)).expect("a managed dxc must satisfy the check");
    }

    #[test]
    fn install_reuses_an_unpacked_copy() {
        let machine = TestMachine::new();
        let dxc = crate::toolchain::managed_tool::dxc();
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
