use num_traits::ToPrimitive as _;
use std::rc::Rc;
use waterui_graphics::gpu_surface::{GpuContext, GpuFrame, GpuView};
use wgpu_external_frame::dma_buf::{DmaBufFrame, DmaBufImporter};

#[cfg(feature = "webview")]
use crate::WpePage;
#[cfg(feature = "webview")]
use crate::input::{WpeInputGpuView, WpeSurfaceInput};

struct SourceTexture {
    size: (u32, u32),
    format: wgpu::TextureFormat,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct GpuState {
    importer: DmaBufImporter,
    target_format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    options: wgpu::Buffer,
    source: Option<SourceTexture>,
}

/// Source of Linux browser frames for GPU-only DMA-BUF composition.
pub trait DmaBufFrameSource: 'static {
    /// Drains engine work that is ready on the current thread.
    fn pump(&self);
    /// Updates the browser viewport.
    fn resize(&self, width: u32, height: u32, scale: f64);
    /// Installs the host redraw callback.
    fn set_frame_waker(&self, waker: Rc<dyn Fn()>);
    /// Takes the newest available frame.
    fn take_frame(&self) -> Option<DmaBufFrame>;
}

#[cfg(feature = "webview")]
impl DmaBufFrameSource for WpePage {
    fn pump(&self) {
        Self::pump(self);
    }

    fn resize(&self, width: u32, height: u32, scale: f64) {
        Self::resize(self, width, height, scale);
    }

    fn set_frame_waker(&self, waker: Rc<dyn Fn()>) {
        Self::set_frame_waker(self, move || waker());
    }

    fn take_frame(&self) -> Option<DmaBufFrame> {
        Self::take_frame(self)
    }
}

/// GPU view that composites a Linux browser DMA-BUF stream without CPU readback.
pub struct DmaBufGpuView<S> {
    source: S,
    gpu: Option<GpuState>,
    pending_frame: Option<DmaBufFrame>,
}

/// WPE-specialized DMA-BUF GPU view.
#[cfg(feature = "webview")]
pub type WpeGpuView = DmaBufGpuView<WpePage>;

impl<S> core::fmt::Debug for DmaBufGpuView<S> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("WpeGpuView").finish_non_exhaustive()
    }
}

impl<S: DmaBufFrameSource> DmaBufGpuView<S> {
    /// Creates a renderer for `source`.
    ///
    /// The device scale comes from the frame the host draws — see
    /// [`GpuFrame::scale`] — so nothing has to publish it separately.
    #[must_use]
    pub const fn new(source: S) -> Self {
        Self {
            source,
            gpu: None,
            pending_frame: None,
        }
    }

    /// Returns the frame source.
    #[must_use]
    pub const fn source(&self) -> &S {
        &self.source
    }
}

/// Creates the presenter for one visible WPE page, wired to take its own input.
///
/// The view reports
/// [`wants_input_events`](waterui_graphics::gpu_surface::GpuView::wants_input_events),
/// so a backend that routes surface input to GPU views needs nothing
/// WPE-specific: the pointer, keyboard, scroll and composition events landing
/// on this layer reach `WPEPlatform` through
/// [`WpeSurfaceInput`](crate::WpeSurfaceInput). A backend whose input arrives
/// somewhere else entirely — GTK delivers it to the `GtkGLArea`'s event
/// controllers — builds a [`DmaBufGpuView`] and owns a `WpeSurfaceInput` beside
/// it instead.
#[cfg(feature = "webview")]
#[must_use]
pub fn gpu_view_with_input(page: WpePage) -> impl GpuView {
    WpeInputGpuView::new(DmaBufGpuView::new(page.clone()), WpeSurfaceInput::new(page))
}

impl<S: DmaBufFrameSource> GpuView for DmaBufGpuView<S> {
    #[expect(
        clippy::future_not_send,
        reason = "browser GPU views and WaterUI environments are confined to the UI thread"
    )]
    async fn setup(&mut self, context: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        let redraw = context.redraw_handle.clone();
        self.source
            .set_frame_waker(Rc::new(move || redraw.request_redraw()));
        self.gpu = Some(create_gpu_state(context));
    }

    fn render(&mut self, frame: &mut GpuFrame<'_>) {
        self.source.pump();
        resize_browser_source(&self.source, frame);
        if self.pending_frame.is_none() {
            self.pending_frame = self.source.take_frame();
        }
        let Some(pending) = self.pending_frame.as_ref() else {
            clear_target(frame);
            frame.request_redraw();
            return;
        };
        if !pending.is_render_ready() {
            frame.request_redraw();
            return;
        }

        let incoming = self
            .pending_frame
            .take()
            .expect("ready WPE frame must remain pending");
        let gpu = self
            .gpu
            .as_mut()
            .expect("WPE GPU view rendered before setup");
        render_browser_frame(gpu, incoming, frame);
        if let Some(next) = self.source.take_frame() {
            self.pending_frame = Some(next);
            frame.request_redraw();
        }
    }
}

fn resize_browser_source<S: DmaBufFrameSource>(source: &S, frame: &GpuFrame<'_>) {
    let scale = frame.scale();
    let logical_width = (f64::from(frame.width) / scale)
        .round()
        .max(1.0)
        .to_u32()
        .expect("WPE logical width exceeds u32");
    let logical_height = (f64::from(frame.height) / scale)
        .round()
        .max(1.0)
        .to_u32()
        .expect("WPE logical height exceeds u32");
    source.resize(logical_width, logical_height, scale);
}

fn render_browser_frame(gpu: &mut GpuState, mut incoming: DmaBufFrame, frame: &GpuFrame<'_>) {
    assert_eq!(
        frame.format, gpu.target_format,
        "WPE target format changed after setup"
    );
    ensure_source_texture(
        gpu,
        frame.device,
        incoming.width,
        incoming.height,
        incoming.format.texture_format(),
    );
    let bind_group = create_source_bind_group(gpu, &incoming, frame.device, frame.queue);
    let source = gpu
        .source
        .as_ref()
        .expect("WPE source texture must exist before import");
    let import = gpu.importer.copy_into(&mut incoming, &source.texture);
    let mut encoder = import.encoder;
    let guard = import.guard;
    incoming.presented();
    encode_browser_blit(gpu, &bind_group, frame, &mut encoder);
    frame.queue.submit([encoder.finish()]);
    frame.queue.on_submitted_work_done(move || {
        drop(guard);
        incoming.release(None);
    });
}

fn create_source_bind_group(
    gpu: &GpuState,
    incoming: &DmaBufFrame,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> wgpu::BindGroup {
    let source = gpu
        .source
        .as_ref()
        .expect("WPE source texture must exist after allocation");
    let force_opaque = u32::from(incoming.format.force_opaque());
    let mut options = [0u8; 16];
    options[..4].copy_from_slice(&force_opaque.to_ne_bytes());
    queue.write_buffer(&gpu.options, 0, &options);
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("waterui_wpe_bind_group"),
        layout: &gpu.bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Sampler(&gpu.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&source.view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: gpu.options.as_entire_binding(),
            },
        ],
    })
}

fn encode_browser_blit(
    gpu: &GpuState,
    bind_group: &wgpu::BindGroup,
    frame: &GpuFrame<'_>,
    encoder: &mut wgpu::CommandEncoder,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("waterui_wpe_blit"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &frame.view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(&gpu.pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.draw(0..6, 0..1);
}

fn create_gpu_state(context: &GpuContext<'_>) -> GpuState {
    let importer = DmaBufImporter::new(context.device, context.queue, context.adapter);
    let shader = context
        .device
        .create_shader_module(wgpu::include_wgsl!("wpe_blit.wgsl"));
    let bind_group_layout =
        context
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("waterui_wpe_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: Some(
                                std::num::NonZeroU64::new(16)
                                    .expect("WPE options size is non-zero"),
                            ),
                        },
                        count: None,
                    },
                ],
            });
    let pipeline_layout = context
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("waterui_wpe_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
    let pipeline = context
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("waterui_wpe_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: context.surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
    GpuState {
        importer,
        target_format: context.surface_format,
        pipeline,
        bind_group_layout,
        sampler: context.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("waterui_wpe_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        }),
        options: context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("waterui_wpe_options"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        source: None,
    }
}

fn ensure_source_texture(
    gpu: &mut GpuState,
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) {
    if gpu
        .source
        .as_ref()
        .is_some_and(|source| source.size == (width, height) && source.format == format)
    {
        return;
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("waterui_wpe_source"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    gpu.source = Some(SourceTexture {
        size: (width, height),
        format,
        texture,
        view,
    });
}

fn clear_target(frame: &GpuFrame<'_>) {
    let mut encoder = frame
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("waterui_wpe_empty_encoder"),
        });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("waterui_wpe_empty"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &frame.view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    frame.queue.submit([encoder.finish()]);
}
