//! The space conversion around a filter over an sRGB-encoded scope
//! (`space_convert.wgsl`). The scope stores premultiplied encoded sRGB
//! while a filter works in the premultiplied linear working space, so the
//! scope's texels are decoded into the filter's input and its output is
//! encoded back, each a texel-for-texel pass in place of a plain copy.

/// The conversion module's layout and its pipelines by target format,
/// built on the first frame that filters an sRGB-encoded scope.
pub struct Pipelines {
    cache: super::PipelineCache,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    programs: [super::shaders::RenderProgram; 2],
    /// `(format, [decode, encode])`: the filter targets share their
    /// scope's format, the surface's or the scratch format.
    by_format: Vec<(wgpu::TextureFormat, [wgpu::RenderPipeline; 2])>,
}

impl Pipelines {
    /// The layout and module; pipelines are built per format on demand.
    pub fn new(
        device: &wgpu::Device,
        delivery: super::shaders::ShaderDelivery,
        cache: super::PipelineCache,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cherenkov space conversion"),
            entries: &super::layout_entries(super::bindings::SPACE_CONVERT_GROUP0),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cherenkov space conversion"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        Self {
            cache,
            layout,
            pipeline_layout,
            programs: delivery.space_convert_programs(device),
            by_format: Vec::new(),
        }
    }

    /// The bind group reading `source`.
    pub fn bind(&self, device: &wgpu::Device, source: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cherenkov space conversion"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            }],
        })
    }

    /// Decodes the `size` texels `source` binds into `target`'s origin:
    /// premultiplied encoded sRGB to the premultiplied working space.
    pub fn decode(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::BindGroup,
        target: (&wgpu::TextureView, wgpu::TextureFormat),
        size: (u32, u32),
    ) {
        self.run(device, encoder, 0, source, target, size);
    }

    /// Encodes the `size` texels `source` binds into `target`'s origin:
    /// the premultiplied working space to premultiplied encoded sRGB.
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::BindGroup,
        target: (&wgpu::TextureView, wgpu::TextureFormat),
        size: (u32, u32),
    ) {
        self.run(device, encoder, 1, source, target, size);
    }

    /// Draws fragment `entry` (0 decodes, 1 encodes) from `source` into
    /// the `size` region at `target`'s origin.
    #[expect(
        clippy::cast_precision_loss,
        reason = "filter target sizes are well within f32"
    )]
    fn run(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        entry: usize,
        source: &wgpu::BindGroup,
        (view, format): (&wgpu::TextureView, wgpu::TextureFormat),
        (width, height): (u32, u32),
    ) {
        let index = if let Some(index) = self.by_format.iter().position(|(f, _)| *f == format) {
            index
        } else {
            let pipelines = self.programs.each_ref().map(|program| {
                super::projective::quad_pipeline(
                    device,
                    &self.pipeline_layout,
                    program,
                    format,
                    None,
                    &self.cache,
                )
            });
            self.by_format.push((format, pipelines));
            self.by_format.len() - 1
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cherenkov space conversion"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                // The target can be larger than the converted region —
                // a scope's scratch — and keeps its texels outside it.
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, width, height);
        pass.set_pipeline(&self.by_format[index].1[entry]);
        pass.set_bind_group(0, source, &[]);
        pass.draw(0..3, 0..1);
    }
}
