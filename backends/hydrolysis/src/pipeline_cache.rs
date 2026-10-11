//! The persistent pipeline-cache path handed to the engine's
//! [`GpuConfig::pipeline_cache`](cherenkov_gpu::GpuConfig): the driver's
//! compiled pipelines are serialised under the platform cache directory on
//! one launch and fed back to pipeline creation on the next, so creation
//! skips the driver's compilation work. On lavapipe that compilation is the
//! dominant cost of a cold launch; on hardware drivers it is the pipeline
//! creation time the vendor's own cache would normally absorb.

use std::path::PathBuf;

/// The file the engine's pipeline cache for `adapter` persists to, or `None`
/// where no writable cache directory exists. The filename carries wgpu's
/// adapter key so a driver change or a different GPU reads no stale blob.
pub fn path(adapter: &wgpu::Adapter) -> Option<PathBuf> {
    let key = wgpu::util::pipeline_cache_key(&adapter.get_info())?;
    let base = std::env::var_os("HYDROLYSIS_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(default_cache_base)?;
    Some(base.join("hydrolysis").join(format!("pipelines-{key}.bin")))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn default_cache_base() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
}

#[cfg(target_os = "macos")]
fn default_cache_base() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches"))
}

#[cfg(windows)]
fn default_cache_base() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}
