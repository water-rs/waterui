//! On-demand compilation and caching of the iOS Simulator headless preview
//! helper binaries under `~/.water/build_cache/ios-sim-preview/<hash>/`.

use std::ffi::OsStr;
#[cfg(unix)]
use std::fs::Permissions;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process;

use eyre::{Context as _, Result};
use futures_util::try_join;
use hex::encode;
use sha2::{Digest as _, Sha256};
use smol::fs;
use tempfile::Builder;

use crate::toolchain::Host;
use crate::water_dir;

use super::runtime::{IosSimRuntime, sdk_path};

const SIMNS_PROTO_H: &str = include_str!("sources/simns_proto.h");
const SIMHOST_M: &str = include_str!("sources/simhost.m");
const SIMNS_C: &str = include_str!("sources/simns.c");
const SIMFB_M: &str = include_str!("sources/simfb.m");
const SIM_INTERPOSE_M: &str = include_str!("sources/sim_interpose.m");

/// Paths to the cached iOS Simulator headless preview helper artifacts.
#[derive(Debug, Clone)]
pub struct IosSimHelpers {
    /// macOS arm64 host process serving `IOSurfaceRemoteServer` + `MTLSimImplementation`.
    pub simhost: PathBuf,
    /// iOS Simulator arm64 Mach bootstrap namespace server.
    pub simns: PathBuf,
    /// iOS Simulator arm64 `SimFramebuffer` dylib (`SIMULATOR_FRAMEBUFFER_FRAMEWORK`).
    pub simfb: PathBuf,
    /// iOS Simulator arm64 interposer dylib (`BKS`/`FBS` hooks + on-demand `MTLCompilerService`).
    pub interpose: PathBuf,
}

impl IosSimHelpers {
    fn in_dir(dir: &Path) -> Self {
        Self {
            simhost: dir.join("waterui-simhost"),
            simns: dir.join("waterui-simns"),
            simfb: dir.join("SimFramebuffer"),
            interpose: dir.join("libwaterui_sim_interpose.dylib"),
        }
    }

    fn all_exist(&self) -> bool {
        self.simhost.is_file()
            && self.simns.is_file()
            && self.simfb.is_file()
            && self.interpose.is_file()
    }
}

/// Ensure the helper binaries for `runtime` are compiled, ad-hoc signed, and
/// cached under `~/.water/build_cache/ios-sim-preview/<hash>/`.
///
/// # Errors
///
/// Returns an error if the Xcode SDKs cannot be located, `clang` or `codesign`
/// fails, or the cache directory cannot be written.
pub async fn ensure_helpers(host: &Host, runtime: &IosSimRuntime) -> Result<IosSimHelpers> {
    let (macos_sdk, sim_sdk, cache_root) = try_join!(
        sdk_path(host, "macosx"),
        sdk_path(host, "iphonesimulator"),
        water_dir::build_cache_root(host),
    )?;

    let key = cache_key(runtime, &macos_sdk, &sim_sdk);
    let parent_dir = cache_root.join("ios-sim-preview");
    let target_dir = parent_dir.join(&key);
    let helpers = IosSimHelpers::in_dir(&target_dir);
    if helpers.all_exist() {
        return Ok(helpers);
    }

    fs::create_dir_all(&parent_dir)
        .await
        .wrap_err_with(|| format!("Failed to create {}", parent_dir.display()))?;

    let staging = Builder::new()
        .prefix("staging-")
        .tempdir_in(&parent_dir)
        .wrap_err("Failed to create staging directory for iOS Simulator preview helpers")?;
    let stage_dir = staging.path();

    fs::write(stage_dir.join("simns_proto.h"), SIMNS_PROTO_H).await?;
    fs::write(stage_dir.join("simhost.m"), SIMHOST_M).await?;
    fs::write(stage_dir.join("simns.c"), SIMNS_C).await?;
    fs::write(stage_dir.join("simfb.m"), SIMFB_M).await?;
    fs::write(stage_dir.join("sim_interpose.m"), SIM_INTERPOSE_M).await?;

    let staged = IosSimHelpers::in_dir(stage_dir);
    try_join!(
        compile_simhost(host, stage_dir, &staged.simhost),
        compile_simns(host, stage_dir, &staged.simns),
        compile_simfb(host, stage_dir, &staged.simfb),
        compile_interpose(host, stage_dir, &staged.interpose),
    )?;

    try_join!(
        ad_hoc_sign(host, &staged.simhost),
        ad_hoc_sign(host, &staged.simns),
        ad_hoc_sign(host, &staged.simfb),
        ad_hoc_sign(host, &staged.interpose),
    )?;

    fs::create_dir_all(&target_dir)
        .await
        .wrap_err_with(|| format!("Failed to create {}", target_dir.display()))?;

    for (src, dst) in [
        (&staged.simhost, &helpers.simhost),
        (&staged.simns, &helpers.simns),
        (&staged.simfb, &helpers.simfb),
        (&staged.interpose, &helpers.interpose),
    ] {
        install_atomic(src, dst).await?;
    }

    Ok(helpers)
}

fn cache_key(runtime: &IosSimRuntime, macos_sdk: &Path, sim_sdk: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"waterui-ios-sim-preview-v1\0");
    hasher.update(runtime.build_version.as_bytes());
    hasher.update(b"\0");
    hasher.update(macos_sdk.to_string_lossy().as_bytes());
    hasher.update(b"\0");
    hasher.update(sim_sdk.to_string_lossy().as_bytes());
    hasher.update(b"\0");
    hasher.update(SIMNS_PROTO_H.as_bytes());
    hasher.update(SIMHOST_M.as_bytes());
    hasher.update(SIMNS_C.as_bytes());
    hasher.update(SIMFB_M.as_bytes());
    hasher.update(SIM_INTERPOSE_M.as_bytes());
    let digest = encode(hasher.finalize());
    digest[..16].to_string()
}

async fn compile_simhost(host: &Host, stage_dir: &Path, output: &Path) -> Result<()> {
    let src = stage_dir.join("simhost.m");
    host.run(
        "xcrun",
        [
            OsStr::new("-sdk"),
            OsStr::new("macosx"),
            OsStr::new("clang"),
            OsStr::new("-arch"),
            OsStr::new("arm64"),
            OsStr::new("-O2"),
            OsStr::new("-fobjc-arc"),
            OsStr::new("-I"),
            stage_dir.as_os_str(),
            src.as_os_str(),
            OsStr::new("-o"),
            output.as_os_str(),
            OsStr::new("-framework"),
            OsStr::new("Foundation"),
            OsStr::new("-framework"),
            OsStr::new("IOSurface"),
            OsStr::new("-framework"),
            OsStr::new("Metal"),
        ],
    )
    .await
    .wrap_err("Failed to compile waterui-simhost helper")?;
    Ok(())
}

async fn compile_simns(host: &Host, stage_dir: &Path, output: &Path) -> Result<()> {
    let src = stage_dir.join("simns.c");
    host.run(
        "xcrun",
        [
            OsStr::new("-sdk"),
            OsStr::new("iphonesimulator"),
            OsStr::new("clang"),
            OsStr::new("-arch"),
            OsStr::new("arm64"),
            OsStr::new("-O2"),
            OsStr::new("-I"),
            stage_dir.as_os_str(),
            src.as_os_str(),
            OsStr::new("-o"),
            output.as_os_str(),
            OsStr::new("-framework"),
            OsStr::new("CoreFoundation"),
        ],
    )
    .await
    .wrap_err("Failed to compile waterui-simns helper")?;
    Ok(())
}

async fn compile_simfb(host: &Host, stage_dir: &Path, output: &Path) -> Result<()> {
    let src = stage_dir.join("simfb.m");
    host.run(
        "xcrun",
        [
            OsStr::new("-sdk"),
            OsStr::new("iphonesimulator"),
            OsStr::new("clang"),
            OsStr::new("-arch"),
            OsStr::new("arm64"),
            OsStr::new("-O2"),
            OsStr::new("-dynamiclib"),
            OsStr::new("-fobjc-arc"),
            OsStr::new("-install_name"),
            OsStr::new("@rpath/SimFramebuffer"),
            src.as_os_str(),
            OsStr::new("-o"),
            output.as_os_str(),
            OsStr::new("-framework"),
            OsStr::new("Foundation"),
            OsStr::new("-framework"),
            OsStr::new("CoreGraphics"),
            OsStr::new("-framework"),
            OsStr::new("IOSurface"),
            OsStr::new("-framework"),
            OsStr::new("Metal"),
        ],
    )
    .await
    .wrap_err("Failed to compile SimFramebuffer helper")?;
    Ok(())
}

async fn compile_interpose(host: &Host, stage_dir: &Path, output: &Path) -> Result<()> {
    let src = stage_dir.join("sim_interpose.m");
    host.run(
        "xcrun",
        [
            OsStr::new("-sdk"),
            OsStr::new("iphonesimulator"),
            OsStr::new("clang"),
            OsStr::new("-arch"),
            OsStr::new("arm64"),
            OsStr::new("-O2"),
            OsStr::new("-dynamiclib"),
            OsStr::new("-fobjc-arc"),
            OsStr::new("-undefined"),
            OsStr::new("dynamic_lookup"),
            OsStr::new("-install_name"),
            OsStr::new("@rpath/libwaterui_sim_interpose.dylib"),
            src.as_os_str(),
            OsStr::new("-o"),
            output.as_os_str(),
            OsStr::new("-framework"),
            OsStr::new("Foundation"),
        ],
    )
    .await
    .wrap_err("Failed to compile libwaterui_sim_interpose.dylib helper")?;
    Ok(())
}

async fn ad_hoc_sign(host: &Host, path: &Path) -> Result<()> {
    host.run(
        "codesign",
        [
            OsStr::new("-s"),
            OsStr::new("-"),
            OsStr::new("-f"),
            path.as_os_str(),
        ],
    )
    .await
    .wrap_err_with(|| format!("Failed to ad-hoc sign {}", path.display()))?;
    Ok(())
}

async fn install_atomic(src: &Path, dst: &Path) -> Result<()> {
    if dst.is_file() {
        return Ok(());
    }
    let pending = dst.with_extension(format!("pending-{}", process::id()));
    fs::copy(src, &pending)
        .await
        .wrap_err_with(|| format!("Failed to stage {}", dst.display()))?;
    #[cfg(unix)]
    {
        let perms = Permissions::from_mode(0o755);
        fs::set_permissions(&pending, perms).await?;
    }
    if let Err(error) = fs::rename(&pending, dst).await {
        let _ = fs::remove_file(&pending).await;
        if !dst.is_file() {
            return Err(error).wrap_err_with(|| format!("Failed to install {}", dst.display()));
        }
    }
    Ok(())
}
