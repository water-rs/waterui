//! Managed tools: pinned, checksum-verified release archives unpacked under
//! `~/.water/tools` — no package manager required.
//!
//! A [`ManagedTool`] names one upstream release artifact (URL, sha256, and the
//! binary's path inside the archive). [`ManagedTool::install`] unpacks the
//! archive into `~/.water/tools/<name>/<version>/` and returns the directory
//! holding the binary; an already-unpacked install is reused without touching
//! the network. Builds put every installed tool's bin directory on `PATH`
//! through [`managed_tools_path_env`], so nothing depends on the user editing
//! `PATH` by hand.

use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use waterui_assets_core::{AssetError, download_remote_bytes, write_bytes_atomically};

use crate::{
    project_model::water_dir::{HomeDirError, water_home_dir},
    toolchain::Host,
};

/// The compression format of a [`ManagedTool`]'s release archive.
#[derive(Debug, Clone, Copy)]
pub enum ArchiveKind {
    /// A `.zip` archive.
    Zip,
    /// A gzipped `.tar` archive.
    TarGz,
}

/// A pinned release archive that unpacks under `~/.water/tools/<name>/<version>/`.
#[derive(Debug, Clone)]
pub struct ManagedTool {
    /// Directory name under `~/.water/tools`.
    pub name: &'static str,
    /// Version segment of the install directory.
    pub version: &'static str,
    /// Pinned archive URL.
    pub url: String,
    /// Expected sha256 of the archive, lowercase hex.
    pub sha256: &'static str,
    /// Path of the tool binary inside the unpacked tree.
    pub binary: String,
    /// The archive's compression format.
    pub archive: ArchiveKind,
}

impl ManagedTool {
    /// The directory this tool unpacks into: `~/.water/tools/<name>/<version>`.
    ///
    /// # Errors
    /// Returns [`HomeDirError`] when the host declares no home directory.
    pub fn install_dir(&self, host: &Host) -> Result<PathBuf, HomeDirError> {
        Ok(water_home_dir(host)?
            .join("tools")
            .join(self.name)
            .join(self.version))
    }

    /// The unpacked binary's path on `host`, when already installed.
    #[must_use]
    pub fn binary_path(&self, host: &Host) -> Option<PathBuf> {
        let path = self.install_dir(host).ok()?.join(&self.binary);
        path.is_file().then_some(path)
    }

    /// The directory holding the binary on `host`, when already installed.
    ///
    /// `binary` is always a multi-component relative path, so `None` parents
    /// collapse to `None` rather than a panic.
    #[must_use]
    pub fn binary_dir(&self, host: &Host) -> Option<PathBuf> {
        self.binary_path(host)
            .and_then(|path| path.parent().map(Path::to_path_buf))
    }

    /// Download the pinned archive, verify its sha256, and unpack it into
    /// `~/.water/tools/<name>/<version>/`. An already-unpacked install is
    /// returned without any network access.
    ///
    /// Returns the directory that holds the tool binary.
    ///
    /// # Errors
    /// Returns [`ManagedToolError`] when the download fails, the checksum does
    /// not match the pinned value, the archive cannot be unpacked, or the
    /// unpacked tree does not contain the expected binary.
    pub async fn install(&self, host: &Host) -> Result<PathBuf, ManagedToolError> {
        if let Some(dir) = self.binary_dir(host) {
            return Ok(dir);
        }

        let tool_dir = water_home_dir(host)?.join("tools").join(self.name);
        let install_dir = tool_dir.join(self.version);
        smol::unblock({
            let tool_dir = tool_dir.clone();
            move || std::fs::create_dir_all(&tool_dir)
        })
        .await?;

        let staging = smol::unblock({
            move || {
                tempfile::Builder::new()
                    .prefix(".staging-")
                    .tempdir_in(&tool_dir)
            }
        })
        .await?;
        let archive_path = staging.path().join("archive");
        fetch_pinned(&self.url, self.sha256, &archive_path).await?;

        let extract_dir = staging.path().join("extract");
        let archive = self.archive;
        smol::unblock({
            let archive_path = archive_path.clone();
            let extract_dir = extract_dir.clone();
            move || -> Result<(), ManagedToolError> {
                std::fs::create_dir_all(&extract_dir)?;
                let file = std::fs::File::open(&archive_path)?;
                match archive {
                    ArchiveKind::Zip => {
                        zip::ZipArchive::new(file)?.extract(&extract_dir)?;
                    }
                    ArchiveKind::TarGz => {
                        tar::Archive::new(flate2::read::GzDecoder::new(file))
                            .unpack(&extract_dir)?;
                    }
                }
                Ok(())
            }
        })
        .await?;

        remove_directory_if_exists(&install_dir).await?;
        smol::unblock({
            let install_dir = install_dir.clone();
            move || std::fs::rename(&extract_dir, &install_dir)
        })
        .await?;

        // Tar archives can carry non-executable modes; guarantee the shipped
        // binary can run.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let binary_path = install_dir.join(&self.binary);
            if binary_path.is_file() {
                smol::unblock(move || {
                    let mut permissions = std::fs::metadata(&binary_path)?.permissions();
                    permissions.set_mode(permissions.mode() | 0o111);
                    std::fs::set_permissions(&binary_path, permissions)
                })
                .await?;
            }
        }

        self.binary_dir(host)
            .ok_or_else(|| ManagedToolError::ArchiveLayout {
                name: self.name,
                binary: self.binary.clone(),
            })
    }
}

/// `dxc` (DirectX Shader Compiler) from the pinned
/// `microsoft/DirectXShaderCompiler` release.
#[must_use]
pub fn dxc() -> ManagedTool {
    let binary = if cfg!(target_arch = "aarch64") {
        "bin/arm64/dxc.exe"
    } else if cfg!(target_arch = "x86_64") {
        "bin/x64/dxc.exe"
    } else {
        "bin/x86/dxc.exe"
    };
    ManagedTool {
        name: "dxc",
        version: "1.8.2505.1",
        url: "https://github.com/microsoft/DirectXShaderCompiler/releases/download/v1.8.2505.1/dxc_2025_07_14.zip"
            .to_string(),
        sha256: "9ad895a6b039e3a8f8c22a1009f866800b840a74b50db9218d13319e215ea8a4",
        binary: binary.to_string(),
        archive: ArchiveKind::Zip,
    }
}

/// `cmake` for a Windows host without a package manager, from the pinned
/// Kitware release zip.
#[must_use]
pub fn cmake() -> Option<ManagedTool> {
    let (package, sha256) = if cfg!(target_arch = "x86_64") {
        (
            "cmake-4.4.3-windows-x86_64",
            "4d52ebab7193a698651639ed80d8d04fd903358843572cf44c7fd234cb7c26ab",
        )
    } else if cfg!(target_arch = "aarch64") {
        (
            "cmake-4.4.3-windows-arm64",
            "7b410ddd00e24c7250eec7452da2348a4a70437aa87e9cda0a20d6a85662fcff",
        )
    } else if cfg!(target_arch = "x86") {
        (
            "cmake-4.4.3-windows-i386",
            "018024d05e2fc77d386046da87f90345f9beea21e35c5c8ab02fd15421b7da18",
        )
    } else {
        return None;
    };
    Some(ManagedTool {
        name: "cmake",
        version: "4.4.3",
        url: format!("https://github.com/Kitware/CMake/releases/download/v4.4.3/{package}.zip"),
        sha256,
        binary: format!("{package}/bin/cmake.exe"),
        archive: ArchiveKind::Zip,
    })
}

/// `sccache` from the pinned Mozilla release artifact.
///
/// A zip on Windows, a gzipped tar on Linux and macOS. Distribution
/// packages lag the 0.9.0 floor the cache protocol needs (apt carries
/// 0.7.x, Fedora none), so the pinned artifact is the repair wherever
/// upstream publishes one. Upstream publishes no Windows x86 build.
#[must_use]
pub fn sccache() -> Option<ManagedTool> {
    let (triple, sha256, archive) = if cfg!(all(windows, target_arch = "x86_64")) {
        (
            "x86_64-pc-windows-msvc",
            "8965c74d5e8a225244f741e18ad2f3f504f48228dc1bac948fc22761a348363d",
            ArchiveKind::Zip,
        )
    } else if cfg!(all(windows, target_arch = "aarch64")) {
        (
            "aarch64-pc-windows-msvc",
            "205d613fa74a9a0525e41a5ace77b1c71907d5bd4a5e668bad79111776829290",
            ArchiveKind::Zip,
        )
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        (
            "x86_64-unknown-linux-musl",
            "45f1447fbe231e3037bde351ef70677dd212216c8d62ae7ca409fecc4d6acc89",
            ArchiveKind::TarGz,
        )
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        (
            "aarch64-unknown-linux-musl",
            "2b3284d5da3b46a47dc4229e75bb7b88ac4aa99c8d754fb7d2f84997e5a4354a",
            ArchiveKind::TarGz,
        )
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "arm",
        target_abi = "eabihf"
    )) {
        (
            "armv7-unknown-linux-musleabi",
            "5e1b69e95cee1b19f0d0669eb1b1597f51770fc602e4321adfea99143cac6ce9",
            ArchiveKind::TarGz,
        )
    } else if cfg!(all(target_os = "linux", target_arch = "x86")) {
        (
            "i686-unknown-linux-musl",
            "e23e961b549c3c40ac0d504e0d4a63a5da2ef1b44ac253c55bef55e755fdf340",
            ArchiveKind::TarGz,
        )
    } else if cfg!(all(target_os = "linux", target_arch = "riscv64")) {
        (
            "riscv64gc-unknown-linux-musl",
            "ee204961bae9c7033971a7a65e93e66431c8e2ca4af9123330ac3e94afacd4de",
            ArchiveKind::TarGz,
        )
    } else if cfg!(all(target_os = "linux", target_arch = "s390x")) {
        (
            "s390x-unknown-linux-musl",
            "c7e532bc7f2e6e1f27c9087172a95faf3b85672256775fde3e8cf26d9934f4fe",
            ArchiveKind::TarGz,
        )
    } else if cfg!(all(target_os = "linux", target_arch = "loongarch64")) {
        (
            "loongarch64-unknown-linux-musl",
            "2c165dd599675a31be5d0e100e8df2bb22919d75ac711f9060acd96fcb7c6626",
            ArchiveKind::TarGz,
        )
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        (
            "aarch64-apple-darwin",
            "308184519b646f5125289e8515b36f6ca65a13a041923994aebe702348674e8e",
            ArchiveKind::TarGz,
        )
    } else {
        return None;
    };
    let package = format!("sccache-v0.18.0-{triple}");
    let (extension, binary) = match archive {
        ArchiveKind::Zip => ("zip", format!("{package}/sccache.exe")),
        ArchiveKind::TarGz => ("tar.gz", format!("{package}/sccache")),
    };
    Some(ManagedTool {
        name: "sccache",
        version: "0.18.0",
        url: format!(
            "https://github.com/mozilla/sccache/releases/download/v0.18.0/{package}.{extension}"
        ),
        sha256,
        binary,
        archive,
    })
}

/// A JDK (Temurin 21) for a Windows host without a package manager, from the
/// pinned Adoptium release zip.
#[must_use]
pub fn jdk() -> Option<ManagedTool> {
    let (package, sha256) = if cfg!(target_arch = "x86_64") {
        (
            "OpenJDK21U-jdk_x64_windows_hotspot_21.0.12.1_1",
            "f9d6e191ab098c0d416e7d588a24420a8621cd2f4720dab2459b8b7b2d2d8b4e",
        )
    } else if cfg!(target_arch = "aarch64") {
        (
            "OpenJDK21U-jdk_aarch64_windows_hotspot_21.0.12.1_1",
            "ccf2e51f527d542a70ba5794a600d3aac04b4e967950e227834c7566cb1bec7b",
        )
    } else {
        return None;
    };
    Some(ManagedTool {
        name: "jdk",
        version: "21.0.12.1+1",
        url: format!(
            "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.12.1%2B1/{package}.zip"
        ),
        sha256,
        binary: "jdk-21.0.12.1+1/bin/java.exe".to_string(),
        archive: ArchiveKind::Zip,
    })
}

/// Every managed tool the CLI can install.
#[must_use]
pub fn all() -> Vec<ManagedTool> {
    let mut tools = vec![dxc()];
    for tool in [cmake(), sccache(), jdk()].into_iter().flatten() {
        tools.push(tool);
    }
    tools
}

/// The LLVM installer MSI for Windows on ARM64, verified against the pinned
/// sha256 and run through `msiexec`.
pub const LLVM_ARM64_MSI_URL: &str =
    "https://github.com/llvm/llvm-project/releases/download/llvmorg-23.1.1/LLVM-23.1.1-woa64.msi";
/// Pinned sha256 of [`LLVM_ARM64_MSI_URL`], lowercase hex.
pub const LLVM_ARM64_MSI_SHA256: &str =
    "aeb4415a5fcfd488dc0ef69ccb2c85a81163c11960b63b6b295c6c3df5a6317e";

/// Download `url`, verify it against the pinned `sha256`, and write it to
/// `destination` atomically.
///
/// # Errors
/// Returns [`ManagedToolError`] when the download fails, the checksum does not
/// match the pinned value, or the file cannot be written.
pub async fn fetch_pinned(
    url: &str,
    sha256: &str,
    destination: &Path,
) -> Result<(), ManagedToolError> {
    let bytes = download_remote_bytes(url).await?;
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != sha256 {
        return Err(ManagedToolError::Checksum {
            expected: sha256.to_string(),
            actual,
        });
    }
    write_bytes_atomically(destination, &bytes).await?;
    Ok(())
}

/// A `"PATH"` env entry prepending every installed managed tool's bin
/// directory to the host's `PATH`, for builds that resolve tools by name
/// (e.g. `shaderloom` invoking `dxc`).
///
/// Returns `None` when no managed tool is installed.
#[must_use]
pub fn managed_tools_path_env(host: &Host) -> Option<(String, OsString)> {
    let mut entries: Vec<PathBuf> = all()
        .into_iter()
        .filter_map(|tool| tool.binary_dir(host))
        .collect();
    if entries.is_empty() {
        return None;
    }
    entries.extend(host.path_entries());
    let value = std::env::join_paths(entries).ok()?;
    Some(("PATH".to_string(), value))
}

async fn remove_directory_if_exists(path: &Path) -> Result<(), ManagedToolError> {
    if smol::unblock({
        let path = path.to_path_buf();
        move || path.is_dir()
    })
    .await
    {
        smol::unblock({
            let path = path.to_path_buf();
            move || std::fs::remove_dir_all(&path)
        })
        .await?;
    }
    Ok(())
}

/// Errors from managed-tool installs.
#[derive(Debug, thiserror::Error)]
pub enum ManagedToolError {
    /// The asset could not be downloaded or written.
    #[error(transparent)]
    Asset(#[from] AssetError),

    /// The host declares no home directory.
    #[error(transparent)]
    HomeDir(#[from] HomeDirError),

    /// An I/O operation failed.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// The downloaded archive could not be read.
    #[error("Could not unpack the release archive: {0}")]
    Zip(#[from] zip::result::ZipError),

    /// The downloaded archive's sha256 does not match the pinned value.
    #[error(
        "Checksum mismatch for the downloaded archive: expected sha256 {expected}, got {actual}"
    )]
    Checksum {
        /// Pinned sha256, lowercase hex.
        expected: String,
        /// Computed sha256, lowercase hex.
        actual: String,
    },

    /// The unpacked archive does not contain the expected binary.
    #[error("The unpacked {name} archive does not contain the expected binary `{binary}`")]
    ArchiveLayout {
        /// Tool name.
        name: &'static str,
        /// Expected binary path inside the unpacked tree.
        binary: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{dxc, managed_tools_path_env};
    use crate::toolchain::testing::TestMachine;

    #[test]
    fn path_env_prepends_installed_tool_binary_dirs() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let dxc = dxc();
        let binary = dxc
            .install_dir(&host)
            .expect("install dir")
            .join(&dxc.binary);
        std::fs::create_dir_all(binary.parent().expect("binary dir")).expect("create bin dir");
        std::fs::write(&binary, b"").expect("write binary");

        let (key, value) =
            managed_tools_path_env(&host).expect("an installed tool yields a PATH env");
        assert_eq!(key, "PATH");
        let entries: Vec<_> = std::env::split_paths(&value).collect();
        assert_eq!(
            entries.first(),
            Some(&binary.parent().expect("binary dir").to_path_buf()),
            "the managed tool's bin directory must lead the build PATH"
        );
    }

    #[test]
    fn path_env_is_none_without_installed_tools() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        assert!(managed_tools_path_env(&host).is_none());
    }
}
