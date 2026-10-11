//! Backdrop capture levels: the reduce pipeline (`reduce.wgsl`) that
//! halves a pyramid level into the next — level `k` texel `(i, j)` is the
//! mean of level `k − 1` texels `(2i..=2i+1, 2j..=2j+1)`, a partial box
//! at the spec edge averaging the texels present — and the parameters it
//! reads.

use bytemuck::{Pod, Zeroable};

/// The tail of a pass's uniform slot after its `Globals`: the parameters
/// `reduce.wgsl`'s `Reduce` reads.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Params {
    /// The source level's spec extent, in its own texels.
    pub source_extent: [f32; 2],
    /// Padding to the uniform alignment.
    pub pad0: [f32; 2],
    /// Padding to the uniform alignment.
    pub pad1: [f32; 4],
}

/// The reduce parameters of one pyramid step: `source` is the spec
/// extent of the level being halved.
#[expect(
    clippy::cast_precision_loss,
    reason = "capture extents are well within f32"
)]
pub fn params(source: (u32, u32)) -> Params {
    Params {
        source_extent: [source.0 as f32, source.1 as f32],
        ..Params::default()
    }
}

/// One reduce step's cached bind group, valid while the globals buffer
/// and the source view are the ones it binds.
pub struct Bind {
    globals: wgpu::Buffer,
    source: wgpu::TextureView,
    group: wgpu::BindGroup,
}

/// The reduce pipeline's layout and its pipelines by target format,
/// built on the first frame that reduces a levelled capture.
pub struct Pipelines {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    /// `(format, pipeline)`: a level stores its capture's format, the
    /// surface's or the scratch format.
    by_format: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

impl Pipelines {
    /// The layout and module; pipelines are built per format on demand.
    pub fn new(device: &wgpu::Device, delivery: super::shaders::ShaderDelivery) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cherenkov reduce"),
            entries: &super::layout_entries(super::bindings::REDUCE_GROUP0),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cherenkov reduce"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        Self {
            layout,
            pipeline_layout,
            module: delivery.reduce_module(device),
            by_format: Vec::new(),
        }
    }

    /// The reduce pipeline writing `format`.
    pub fn pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        let index = if let Some(index) = self.by_format.iter().position(|(f, _)| *f == format) {
            index
        } else {
            let pipeline = super::projective::quad_pipeline(
                device,
                &self.pipeline_layout,
                &self.module,
                "fs_main",
                format,
                None,
            );
            self.by_format.push((format, pipeline));
            self.by_format.len() - 1
        };
        &self.by_format[index].1
    }

    /// The bind group reading `source` with the parameters in `globals`,
    /// reusing `cache` while it binds the same buffer and view.
    pub fn bind<'a>(
        &self,
        device: &wgpu::Device,
        cache: &'a mut Option<Bind>,
        globals: &wgpu::Buffer,
        source: &wgpu::TextureView,
    ) -> &'a wgpu::BindGroup {
        if cache
            .as_ref()
            .is_none_or(|bind| bind.globals != *globals || bind.source != *source)
        {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cherenkov reduce"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: globals,
                            offset: 0,
                            size: wgpu::BufferSize::new(super::bindings::REDUCE_PARAMS_SIZE),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                ],
            });
            *cache = Some(Bind {
                globals: globals.clone(),
                source: source.clone(),
                group,
            });
        }
        &cache.as_ref().expect("filled above").group
    }
}

/// Draws the reduce into the open `pass` over a `size` viewport: one
/// triangle, the uniform slot at `offset`.
#[expect(
    clippy::cast_precision_loss,
    reason = "capture sizes are well within f32"
)]
pub fn draw(
    pass: &mut wgpu::RenderPass<'_>,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
    offset: u32,
    size: (u32, u32),
) {
    pass.set_viewport(0.0, 0.0, size.0 as f32, size.1 as f32, 0.0, 1.0);
    pass.set_scissor_rect(0, 0, size.0, size.1);
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[offset]);
    pass.draw(0..3, 0..1);
}
