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
use core::cell::RefCell;
use std::collections::HashMap;

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

#[derive(Debug, Default)]
struct Entries {
    vertex: Option<wgpu::ShaderModule>,
    passes: HashMap<PassKey, PassPipeline>,
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

    /// The pipeline already built for `key`.
    pub(crate) fn get(&self, key: &PassKey) -> Option<PassPipeline> {
        self.entries.borrow().passes.get(key).cloned()
    }

    /// Records a validated pipeline. When a concurrent setup of the same
    /// key finished first, its pipeline stays and is returned.
    pub(crate) fn insert(&self, key: PassKey, built: PassPipeline) -> PassPipeline {
        self.entries
            .borrow_mut()
            .passes
            .entry(key)
            .or_insert(built)
            .clone()
    }

    /// How many distinct pass pipelines the cache holds.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.entries.borrow().passes.len()
    }
}
