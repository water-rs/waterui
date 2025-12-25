//! Render context for command fusion.

use std::sync::Arc;

/// Shared context for recording GPU commands without immediate submission.
///
/// `RenderContext` enables command fusion by allowing multiple sources
/// and filters to encode their operations to the same command encoder.
/// The encoder is only submitted when [`Pipeline::render`] is called.
pub struct RenderContext<'a> {
    /// The wgpu device for creating GPU resources.
    pub device: &'a wgpu::Device,
    /// The wgpu queue (for buffer writes, not submission during encoding).
    pub queue: &'a wgpu::Queue,
    /// The shared command encoder for fused execution.
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// Optional pipeline cache for faster pipeline creation.
    pub pipeline_cache: Option<&'a wgpu::PipelineCache>,
}

impl core::fmt::Debug for RenderContext<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RenderContext").finish_non_exhaustive()
    }
}

/// Resources for creating pipelines and textures.
///
/// Used during setup phase before rendering.
#[allow(dead_code)]
pub struct GpuResources {
    /// The wgpu device.
    pub device: Arc<wgpu::Device>,
    /// The wgpu queue.
    pub queue: Arc<wgpu::Queue>,
    /// Optional pipeline cache.
    pub pipeline_cache: Option<Arc<wgpu::PipelineCache>>,
}

impl core::fmt::Debug for GpuResources {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GpuResources").finish_non_exhaustive()
    }
}

#[allow(dead_code)]
impl GpuResources {
    /// Creates a new `GpuResources` from device and queue.
    #[must_use]
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        Self {
            device,
            queue,
            pipeline_cache: None,
        }
    }

    /// Sets the pipeline cache.
    #[must_use]
    pub fn with_cache(mut self, cache: Arc<wgpu::PipelineCache>) -> Self {
        self.pipeline_cache = Some(cache);
        self
    }
}
