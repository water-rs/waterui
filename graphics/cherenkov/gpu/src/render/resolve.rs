//! Reduced-scale backdrop captures: the resolve pipeline (`resolve.wgsl`)
//! that downsamples the device pixels under a capture region onto the
//! capture grid, and the per-pass parameters it reads.

use bytemuck::{Pod, Zeroable};

use super::lower::{Pass, Resolve};

/// The tail of a pass's uniform slot after its `Globals`: the parameters
/// `resolve.wgsl`'s `Resolve` reads.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Params {
    /// The capture region's origin on the capture grid, in texels.
    pub texel_origin: [f32; 2],
    /// The device position of the source texture's texel `(0, 0)`.
    pub source_origin: [f32; 2],
    /// The device extent texel spans clip to.
    pub extent: [f32; 2],
    /// The capture scale.
    pub scale: f32,
    /// Padding to the uniform alignment.
    pub pad: f32,
}

/// The resolve a capture pass runs, when its capture is reduced.
pub fn of(pass: &Pass) -> Option<Resolve> {
    pass.capture.and_then(|capture| capture.resolve)
}

/// Whether a reduced capture pass resolves through the surface's
/// staging texture: its looked-through composites, from levels that
/// painted before the capture, draw over the 1:1 device copy first, so
/// the pass's draws target the staging texture in device space.
pub const fn staged(pass: &Pass) -> bool {
    !pass.ranges.is_empty()
}

/// The resolve parameters of pass `pass`, zero for every pass that runs
/// no resolve.
#[expect(
    clippy::cast_precision_loss,
    reason = "capture and device coordinates are well within f32"
)]
pub fn params(pass: &Pass) -> Params {
    of(pass).map_or_else(Params::default, |resolve| Params {
        texel_origin: [pass.region[0] as f32, pass.region[1] as f32],
        source_origin: if staged(pass) {
            [resolve.device[0] as f32, resolve.device[1] as f32]
        } else {
            [0.0, 0.0]
        },
        extent: [resolve.extent[0] as f32, resolve.extent[1] as f32],
        scale: resolve.scale,
        pad: 0.0,
    })
}

/// The device-space region a pass's draws cover: the shared staging
/// texture's device rect for a staged resolve, the pass's own region
/// otherwise.
pub fn draw_region(pass: &Pass) -> [u32; 4] {
    match of(pass) {
        Some(resolve) if staged(pass) => resolve.device,
        _ => pass.region,
    }
}

/// A cached resolve bind group — a direct region's or a staging
/// slot's — valid while the globals buffer and source view are the
/// ones it binds.
pub struct Bind {
    globals: wgpu::Buffer,
    source: wgpu::TextureView,
    group: wgpu::BindGroup,
}

/// The resolve pipeline's layout and its pipelines by target format,
/// built on the first frame that resolves a reduced capture.
pub struct Pipelines {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    /// `(format, pipeline)`: a capture stores its source's format, the
    /// surface's or the scratch format.
    by_format: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

impl Pipelines {
    /// The layout and module; pipelines are built per format on demand.
    pub fn new(device: &wgpu::Device, delivery: super::shaders::ShaderDelivery) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cherenkov resolve"),
            entries: &super::layout_entries(super::bindings::RESOLVE_GROUP0),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cherenkov resolve"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        Self {
            layout,
            pipeline_layout,
            module: delivery.resolve_module(device),
            by_format: Vec::new(),
        }
    }

    /// The resolve pipeline writing `format`.
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
                label: Some("cherenkov resolve"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: globals,
                            offset: 0,
                            size: wgpu::BufferSize::new(super::bindings::RESOLVE_PARAMS_SIZE),
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

/// Draws the resolve into the open `pass` over a `size` viewport: one
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
