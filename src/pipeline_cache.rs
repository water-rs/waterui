//! Persistent [`wgpu::PipelineCache`]: the driver's compiled pipelines are
//! serialised under the platform cache directory on one launch and fed back to
//! [`wgpu::Device::create_pipeline_cache`] on the next, so pipeline creation
//! skips the driver's compilation work. On lavapipe that compilation is the
//! dominant cost of a cold launch; on hardware drivers it is the pipeline
//! creation time the vendor's own cache would normally absorb.

use std::path::PathBuf;
use std::sync::Arc;

/// A [`wgpu::PipelineCache`] bound to the file it was loaded from.
pub(crate) struct Store {
    cache: wgpu::PipelineCache,
    path: PathBuf,
}

impl Store {
    /// The cache handle renderers hand to `wgpu` pipeline creation.
    pub(crate) fn cache(&self) -> wgpu::PipelineCache {
        self.cache.clone()
    }

    /// Serialise the cache back to its file. The write goes through a sibling
    /// temp file and a rename so a kill mid-write cannot leave a truncated
    /// cache for the next launch.
    pub(crate) fn persist(&self) {
        let Some(data) = self.cache.get_data() else {
            return;
        };
        let tmp = self.path.with_extension("tmp");
        let written = self
            .path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&tmp, &data))
            .and_then(|()| std::fs::rename(&tmp, &self.path));
        if let Err(error) = written {
            tracing::warn!(
                path = %self.path.display(),
                %error,
                "could not persist the pipeline cache; the next launch compiles its pipelines again"
            );
        }
    }

    /// Persist off the caller's thread: `get_data` asks the driver to
    /// serialise every compiled pipeline, which is not a render-loop cost.
    pub(crate) fn persist_on_worker(self: &Arc<Self>) {
        let store = Arc::clone(self);
        std::thread::spawn(move || store.persist());
    }
}

/// The file the pipeline cache for `adapter` persists to, or `None` where no
/// writable cache directory exists. The filename carries wgpu's adapter key so
/// a driver change or a different GPU reads no stale blob — and
/// `fallback: true` below makes even a stale read harmless.
fn store_path(adapter: &wgpu::Adapter) -> Option<PathBuf> {
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

/// Open the persistent pipeline cache for `device`/`adapter`, or `None` where
/// the device was not granted `wgpu::Features::PIPELINE_CACHE`, the adapter
/// reports no cache key, or no writable cache directory exists.
pub(crate) fn open(device: &wgpu::Device, adapter: &wgpu::Adapter) -> Option<Arc<Store>> {
    if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        return None;
    }
    let path = store_path(adapter)?;
    let data = std::fs::read(&path).ok();
    // SAFETY: `data` is only ever a blob a previous `PipelineCache::get_data`
    // wrote to this file; `fallback: true` makes wgpu discard it rather than
    // error when it was produced by another adapter or wgpu version.
    let cache = unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("hydrolysis-pipeline-cache"),
            data: data.as_deref(),
            fallback: true,
        })
    };
    Some(Arc::new(Store { cache, path }))
}
