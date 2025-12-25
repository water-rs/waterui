//! Lazy pipeline for chaining sources and filters.

use crate::{Filter, RenderContext, Source};

/// A lazy pipeline that chains a source with filters.
///
/// The pipeline is built by chaining `.then()` calls, but nothing
/// executes until `.render()` is called. This enables:
///
/// - **Lazy evaluation**: No GPU work until render
/// - **Fusion**: All operations in one `queue.submit()`
/// - **Reuse**: Same pipeline can render to multiple outputs
///
/// # Example
///
/// ```ignore
/// let pipeline = Pipeline::source(my_source)
///     .then(blur)
///     .then(saturation);
///
/// pipeline.render(device, queue, &output);
/// ```
pub struct Pipeline<S: Source> {
    source: S,
    filters: Vec<Box<dyn Filter>>,
}

impl<S: Source> core::fmt::Debug for Pipeline<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Pipeline")
            .field("filters_count", &self.filters.len())
            .finish_non_exhaustive()
    }
}

impl<S: Source> Pipeline<S> {
    /// Creates a new pipeline from a source.
    #[must_use]
    pub fn source(source: S) -> Self {
        Self {
            source,
            filters: Vec::new(),
        }
    }

    /// Adds a filter to the pipeline.
    #[must_use]
    pub fn then<F: Filter + 'static>(mut self, filter: F) -> Self {
        self.filters.push(Box::new(filter));
        self
    }

    /// Renders the pipeline to the output texture.
    ///
    /// This is where all the GPU work happens in a single fused submission.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output: &wgpu::TextureView,
    ) {
        self.render_with_cache(device, queue, None, output);
    }

    /// Renders the pipeline with an optional pipeline cache.
    pub fn render_with_cache(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline_cache: Option<&wgpu::PipelineCache>,
        output: &wgpu::TextureView,
    ) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("filtrate pipeline encoder"),
        });

        {
            let mut ctx = RenderContext {
                device,
                queue,
                encoder: &mut encoder,
                pipeline_cache,
            };

            if self.filters.is_empty() {
                // No filters, just resolve source directly
                let source_view = self.source.resolve(&mut ctx);
                // Copy source to output
                Self::copy_texture(&mut ctx, &source_view, output);
            } else {
                // Create scratch textures for ping-pong
                let (width, height) = self.source.dimensions();
                let scratch_a = Self::create_scratch_texture(device, width, height);
                let scratch_b = Self::create_scratch_texture(device, width, height);
                let scratch_a_view = scratch_a.create_view(&wgpu::TextureViewDescriptor::default());
                let scratch_b_view = scratch_b.create_view(&wgpu::TextureViewDescriptor::default());

                // Resolve source
                let source_view = self.source.resolve(&mut ctx);

                // Apply filters with ping-pong
                let mut current_input = &source_view;
                let mut current_output = &scratch_a_view;
                let mut use_a = true;

                for (i, filter) in self.filters.iter().enumerate() {
                    let is_last = i == self.filters.len() - 1;

                    if is_last {
                        // Last filter writes to final output
                        filter.encode(&mut ctx, current_input, output);
                    } else {
                        filter.encode(&mut ctx, current_input, current_output);
                        // Swap for next iteration
                        current_input = current_output;
                        current_output = if use_a { &scratch_b_view } else { &scratch_a_view };
                        use_a = !use_a;
                    }
                }
            }
        }

        // Single submission for the entire pipeline
        queue.submit([encoder.finish()]);
    }

    fn create_scratch_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("filtrate scratch texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn copy_texture(
        ctx: &mut RenderContext<'_>,
        _src: &wgpu::TextureView,
        _dst: &wgpu::TextureView,
    ) {
        // TODO: Implement texture copy via render pass or compute
        // For now, this is a placeholder - in practice the source
        // should write directly to the output when there are no filters
        let _ = ctx;
    }
}

/// Extension trait for sources to enable ergonomic chaining.
pub trait SourceExt: Source + Sized {
    /// Wraps this source in a pipeline.
    fn pipeline(self) -> Pipeline<Self> {
        Pipeline::source(self)
    }
}

impl<S: Source> SourceExt for S {}
