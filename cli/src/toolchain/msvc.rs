//! Toolchain support for the MSVC C++ build tools — `link.exe` and the C++
//! libraries every Windows-targeting cargo build needs to link its binaries.

use std::path::{Path, PathBuf};

use waterui_assets_core::{AssetError, download_remote_bytes, write_bytes_atomically};

use crate::toolchain::{Host, Installation, Toolchain, ToolchainError};

/// The Visual Studio Build Tools bootstrapper (evergreen aka.ms link — the
/// payload is Microsoft's own Authenticode-signed installer, so no source
/// sha256 exists to pin).
const VS_BUILD_TOOLS_URL: &str = "https://aka.ms/vs/17/release/vs_BuildTools.exe";

/// The Visual Studio component `vswhere` looks for — the MSVC C++ toolset.
const VC_TOOLS_COMPONENT: &str = "Microsoft.VisualStudio.Component.VC.Tools.x86.x64";

/// `0xBC2` (`ERROR_SUCCESS_REBOOT_REQUIRED`): the install succeeded but a
/// reboot is pending — `--norestart` keeps it pending rather than rebooting.
const EXIT_SUCCESS_REBOOT_REQUIRED: i32 = 0xBC2;

/// MSVC C++ build tools on a Windows host.
#[derive(Debug, Clone, Copy, Default)]
pub struct MsvcBuildTools;

impl MsvcBuildTools {
    /// `vswhere.exe` — Microsoft's installer locator, which ships at a fixed
    /// path under `Program Files (x86)` rather than on `PATH`.
    async fn vswhere(host: &Host) -> Option<PathBuf> {
        if let Ok(path) = host.which("vswhere").await {
            return Some(path);
        }
        let program_files_x86 = host.env("ProgramFiles(x86)")?;
        let path = Path::new(&program_files_x86)
            .join("Microsoft Visual Studio")
            .join("Installer")
            .join("vswhere.exe");
        path.is_file().then_some(path)
    }

    /// Whether `vswhere` reports an installation carrying the MSVC C++
    /// toolset.
    async fn vswhere_reports_vc_tools(host: &Host) -> bool {
        let Some(vswhere) = Self::vswhere(host).await else {
            return false;
        };
        let Ok(output) = host
            .output(
                &vswhere,
                [
                    "-products",
                    "*",
                    "-requires",
                    VC_TOOLS_COMPONENT,
                    "-property",
                    "installationPath",
                    "-latest",
                ],
            )
            .await
        else {
            return false;
        };
        output.status.success() && !output.stdout.trim_ascii().is_empty()
    }
}

impl Toolchain for MsvcBuildTools {
    type Installation = MsvcBuildToolsInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if host.which("link.exe").await.is_ok() {
            return Ok(());
        }
        if Self::vswhere_reports_vc_tools(host).await {
            return Ok(());
        }
        Err(ToolchainError::fixable(MsvcBuildToolsInstallation))
    }
}

/// Run the official Visual Studio Build Tools bootstrapper with the
/// `VCTools` workload.
///
/// A system-wide install outside `~/.water`, so the doctor fix loop confirms
/// it with the user first (or proceeds on `--yes`).
#[derive(Debug, Clone, Copy)]
pub struct MsvcBuildToolsInstallation;

/// Errors that can occur while installing the MSVC build tools.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallMsvcBuildTools {
    /// The bootstrapper could not be downloaded or written.
    #[error(transparent)]
    Asset(#[from] AssetError),

    /// An I/O operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// The installer could not be spawned or awaited.
    #[error(transparent)]
    Command(#[from] crate::utils::CommandError),

    /// The bootstrapper ran but reported a failure.
    #[error("Visual Studio Build Tools installer exited with {0}")]
    InstallerFailed(std::process::ExitStatus),

    /// The installer can only run on Windows.
    #[error("Visual Studio Build Tools can only be installed on Windows")]
    UnsupportedPlatform,
}

impl Installation for MsvcBuildToolsInstallation {
    type Error = FailToInstallMsvcBuildTools;

    fn modifies_system(&self) -> bool {
        true
    }

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if !cfg!(target_os = "windows") {
            return Err(FailToInstallMsvcBuildTools::UnsupportedPlatform);
        }
        let bytes = download_remote_bytes(VS_BUILD_TOOLS_URL).await?;
        let staging = smol::unblock(tempfile::tempdir).await?;
        let bootstrapper = staging.path().join("vs_BuildTools.exe");
        write_bytes_atomically(&bootstrapper, &bytes).await?;
        let output = host
            .output(
                &bootstrapper,
                [
                    "--quiet",
                    "--wait",
                    "--norestart",
                    "--add",
                    "Microsoft.VisualStudio.Workload.VCTools",
                    "--includeRecommended",
                ],
            )
            .await?;
        if output.status.success() || output.status.code() == Some(EXIT_SUCCESS_REBOOT_REQUIRED) {
            return Ok(());
        }
        Err(FailToInstallMsvcBuildTools::InstallerFailed(output.status))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::MsvcBuildTools;
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

    #[test]
    fn missing_reports_fixable() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(MsvcBuildTools.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "a host without MSVC build tools must report fixable: {result:?}"
        );
    }

    #[test]
    fn ok_when_link_exe_on_path() {
        let machine = TestMachine::new();
        // `install("link.exe")` would produce `link.exe.cmd` on Windows,
        // which `which("link.exe")` never resolves — the probe needs the
        // literal `.exe` name, so stage it via `executable` instead.
        machine.executable(Path::new("bin").join("link.exe"));
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(MsvcBuildTools.check(&host)).expect("link.exe on PATH must be ok");
    }

    /// A `vswhere` reachable on `PATH` that reports an install location —
    /// works on every platform because the fake dispatcher answers by name.
    #[test]
    fn ok_when_vswhere_reports_vc_tools() {
        let machine = TestMachine::new();
        machine.install("vswhere");
        machine.respond(
            "VSWHERE",
            "C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools",
        );
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(MsvcBuildTools.check(&host))
            .expect("vswhere reporting a VC.Tools install must be ok");
    }

    /// The canonical `ProgramFiles(x86)` probe. On Windows a `.exe` fixture
    /// cannot carry the shell dispatcher (same limitation as `adb.exe`), so
    /// the spawn-failure path is exercised there instead.
    #[test]
    #[cfg(unix)]
    fn ok_when_vswhere_at_installer_path_reports_vc_tools() {
        let machine = TestMachine::new();
        let program_files = machine.dir("Program Files (x86)");
        machine.executable(
            Path::new("Program Files (x86)")
                .join("Microsoft Visual Studio")
                .join("Installer")
                .join("vswhere.exe"),
        );
        machine.respond(
            "VSWHERE",
            "C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools",
        );
        let host = machine.host([("ProgramFiles(x86)", program_files.as_os_str())]);
        smol::block_on(MsvcBuildTools.check(&host))
            .expect("a vswhere at the installer path reporting VC.Tools must be ok");
    }

    #[test]
    fn missing_when_vswhere_finds_nothing() {
        let machine = TestMachine::new();
        machine.install("vswhere");
        // No VSWHERE response staged → vswhere prints nothing → no VC tools.
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(MsvcBuildTools.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "vswhere with no VC.Tools install must report fixable: {result:?}"
        );
    }
}
