//! Render pipelines shared between effects whose compiled result is identical.
//!
//! Building a pipeline compiles its shaders on the backend, which dwarfs
//! everything else in an effect's setup. Two effects whose passes have the
//! same shader, bind group layout and target format compile the same
//! pipeline, so the consumer owns one [`PipelineCache`] per wgpu device and
//! hands it to every [`EffectContext`](crate::EffectContext); the cost is then
//! per distinct chain rather than per instance.

extern crate alloc;

use alloc::{borrow::Cow, string::String, vec::Vec};
use core::{cell::RefCell, fmt, future::Future};
use std::collections::HashMap;

#[cfg(not(target_family = "wasm"))]
use futures_util::future::BoxFuture;
#[cfg(target_family = "wasm")]
use futures_util::future::LocalBoxFuture;
use futures_util::future::{FutureExt, Shared};

/// The full-screen triangle every pass draws.
const VERTEX_SHADER: &str = include_str!("shaders/fullscreen.wgsl");

/// Everything that determines a pass's compiled pipeline.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct PassKey {
    /// The pass module's WGSL: the fragment shader, entry point and bindings.
    pub wgsl: String,
    /// The bind group layout; filterability and sampler kind are not part of
    /// the WGSL.
    pub layout: Vec<wgpu::BindGroupLayoutEntry>,
    /// Whether the pass resizes its output. It is already visible in the WGSL
    /// and the layout; the key states it so the cache never depends on that.
    pub resizes_output: bool,
    /// The color target's format.
    pub format: wgpu::TextureFormat,
}

/// A pass's pipeline with the layout its bind groups are created against.
#[derive(Debug, Clone)]
pub struct PassPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub layout: wgpu::BindGroupLayout,
}

/// A pass pipeline's compilation, resolving to the pipeline or the
/// validation diagnostic. Natively it is `Send`, as the cache that holds it
/// has always been; a page's wgpu handles are not.
#[cfg(not(target_family = "wasm"))]
pub type PassCompile = BoxFuture<'static, Result<PassPipeline, String>>;
/// A pass pipeline's compilation, resolving to the pipeline or the
/// validation diagnostic.
#[cfg(target_family = "wasm")]
pub type PassCompile = LocalBoxFuture<'static, Result<PassPipeline, String>>;

/// Erases a pass compilation, mapping its validation error to the diagnostic.
#[cfg(not(target_family = "wasm"))]
pub fn pass_compile(
    compile: impl Future<Output = Result<PassPipeline, wgpu::Error>> + Send + 'static,
) -> PassCompile {
    compile
        .map(|built| built.map_err(|error| error.to_string()))
        .boxed()
}

/// Erases a pass compilation, mapping its validation error to the diagnostic.
#[cfg(target_family = "wasm")]
pub fn pass_compile(
    compile: impl Future<Output = Result<PassPipeline, wgpu::Error>> + 'static,
) -> PassCompile {
    compile
        .map(|built| built.map_err(|error| error.to_string()))
        .boxed_local()
}

/// One pass pipeline's compilation, shared by every setup that asked for
/// its key while it ran and by every later one. A failure carries the
/// validation diagnostic; each waiter reports it under its own pass index.
pub type PassBuild = Shared<PassCompile>;

#[derive(Default)]
struct Entries {
    vertex: Option<wgpu::ShaderModule>,
    passes: HashMap<PassKey, PassBuild>,
}

impl fmt::Debug for Entries {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Entries")
            .field("vertex", &self.vertex)
            .field("passes", &self.passes.len())
            .finish()
    }
}

/// The pipelines and shader modules effects share on one wgpu device.
///
/// The cache belongs to a device: its handles are invalid on any other, so a
/// consumer keeps one for as long as it keeps the device and drops it with
/// it. Setup runs on the GPU host thread, so the cache is single-threaded
/// (`RefCell`, never held across an await).
#[derive(Debug, Default)]
pub struct PipelineCache {
    entries: RefCell<Entries>,
    driver: Option<wgpu::PipelineCache>,
}

impl PipelineCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Shares the caller's device-bound driver compilation cache.
    #[must_use]
    pub fn with_driver_cache(driver: Option<wgpu::PipelineCache>) -> Self {
        Self {
            driver,
            ..Self::default()
        }
    }

    pub(crate) const fn driver(&self) -> Option<&wgpu::PipelineCache> {
        self.driver.as_ref()
    }

    /// The full-screen triangle vertex module, created on first use.
    pub(crate) fn vertex(&self, device: &wgpu::Device) -> wgpu::ShaderModule {
        self.entries
            .borrow_mut()
            .vertex
            .get_or_insert_with(|| {
                device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("filtrate full-screen triangle"),
                    source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(VERTEX_SHADER)),
                })
            })
            .clone()
    }

    /// The compilation of `key`'s pipeline: the one already built or in
    /// flight, or a new one `build` starts. Concurrent setups of identical
    /// passes therefore compile once.
    pub(crate) fn pass(
        &self,
        key: PassKey,
        build: impl FnOnce(&PassKey) -> PassCompile,
    ) -> PassBuild {
        let existing = self.entries.borrow().passes.get(&key).cloned();
        if let Some(existing) = existing {
            return existing;
        }
        // `build` reads the cache (the vertex module, the driver cache), so
        // no borrow is held across it.
        let started = FutureExt::shared(build(&key));
        self.entries
            .borrow_mut()
            .passes
            .insert(key, started.clone());
        started
    }

    /// Drops every compilation that failed, so a later setup of the same
    /// key compiles again instead of inheriting the error.
    pub(crate) fn forget_failed(&self) {
        self.entries
            .borrow_mut()
            .passes
            .retain(|_, build| !matches!(build.peek(), Some(Err(_))));
    }

    /// How many distinct pass pipelines the cache holds.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.entries.borrow().passes.len()
    }
}
