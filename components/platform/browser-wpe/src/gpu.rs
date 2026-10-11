use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use num_traits::ToPrimitive as _;
use waterui_graphics::gpu::{Context, Frame, GpuContent, GpuContentView, RedrawHandle};
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

struct Mailbox {
    /// Browser frames the UI side drained from the source and the render side
    /// has not yet consumed.
    frames: VecDeque<DmaBufFrame>,
    /// The physical size and device-pixel ratio of the last rendered frame,
    /// published to the UI side so it can keep the browser's logical viewport
    /// in step. `(0, 0, _)` marks "no frame yet".
    viewport: (u32, u32, f64),
    /// The render thread's redraw handle, published once at setup so the UI
    /// side can hand it to the frame source as its waker.
    redraw: Option<RedrawHandle>,
}

/// Crosses the UI/render boundary both ways between [`DmaBufUiBridge`] and
/// [`DmaBufGpuContent`].
struct DmaBufShared {
    state: Mutex<Mailbox>,
}

impl DmaBufShared {
    const fn new() -> Self {
        Self {
            state: Mutex::new(Mailbox {
                frames: VecDeque::new(),
                viewport: (0, 0, 1.0),
                redraw: None,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Mailbox> {
        self.state.lock().expect("WPE frame mailbox poisoned")
    }
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

/// The render-side half of a DMA-BUF browser view: owns the GPU state and the
/// newest pending frame, all `Send`.
struct DmaBufGpuContent {
    shared: Arc<DmaBufShared>,
    gpu: Option<GpuState>,
    pending_frame: Option<DmaBufFrame>,
}

impl GpuContent for DmaBufGpuContent {
    fn setup(&mut self, context: &Context<'_>) {
        self.shared.lock().redraw = Some(context.redraw.clone());
        self.gpu = Some(create_gpu_state(context));
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        {
            let mut state = self.shared.lock();
            state.viewport = (frame.width, frame.height, f64::from(frame.scale));
            if self.pending_frame.is_none() {
                self.pending_frame = state.frames.pop_front();
            }
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
            .expect("WPE GPU content rendered before setup");
        render_browser_frame(gpu, incoming, frame);
        let next = self.shared.lock().frames.pop_front();
        if let Some(next) = next {
            self.pending_frame = Some(next);
            frame.request_redraw();
        }
    }
}

/// The UI-thread half of a DMA-BUF browser view: drives the frame source and
/// hands its frames to the render side.
///
/// `WpePage` and any other [`DmaBufFrameSource`] are confined to the UI
/// thread, so everything that touches the source lives here and only DMA-BUF
/// frames cross.
struct DmaBufUiBridge<S> {
    source: S,
    shared: Arc<DmaBufShared>,
    waker_installed: bool,
}

impl<S: DmaBufFrameSource> DmaBufUiBridge<S> {
    /// Runs one UI-side browser frame; call once per presented frame.
    ///
    /// # Panics
    ///
    /// Panics when the logical viewport does not fit a `u32`.
    fn frame(&mut self) {
        if !self.waker_installed {
            let redraw = self.shared.lock().redraw.take();
            if let Some(redraw) = redraw {
                self.source
                    .set_frame_waker(Rc::new(move || redraw.request_redraw()));
                self.waker_installed = true;
            }
        }
        self.source.pump();
        let (width, height, scale) = self.shared.lock().viewport;
        if width > 0 && height > 0 {
            let logical_width = (f64::from(width) / scale)
                .round()
                .max(1.0)
                .to_u32()
                .expect("WPE logical width exceeds u32");
            let logical_height = (f64::from(height) / scale)
                .round()
                .max(1.0)
                .to_u32()
                .expect("WPE logical height exceeds u32");
            self.source.resize(logical_width, logical_height, scale);
        }
        while let Some(next) = self.source.take_frame() {
            self.shared.lock().frames.push_back(next);
        }
    }
}

/// GPU view that composites a Linux browser DMA-BUF stream without CPU
/// readback.
///
/// The source stays on the UI thread — [`DmaBufFrameSource`] implementations
/// are browser engine objects — so the produced [`GpuContentView`] carries a
/// frame hook that drives it; only the frames themselves cross to the render
/// side.
pub struct DmaBufGpuView<S> {
    source: S,
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
    /// Creates a view over `source`'s frame stream.
    ///
    /// The device scale comes from the frame the host draws — see
    /// [`Frame::scale`] — so nothing has to publish it separately.
    #[must_use]
    pub const fn new(source: S) -> Self {
        Self { source }
    }

    /// Returns the frame source.
    #[must_use]
    pub const fn source(&self) -> &S {
        &self.source
    }

    /// Builds the [`GpuContentView`] that composites the frame stream.
    #[must_use]
    pub fn into_view(self) -> GpuContentView {
        let shared = Arc::new(DmaBufShared::new());
        let content = DmaBufGpuContent {
            shared: Arc::clone(&shared),
            gpu: None,
            pending_frame: None,
        };
        let bridge = RefCell::new(DmaBufUiBridge {
            source: self.source,
            shared,
            waker_installed: false,
        });
        GpuContentView::new(content).on_frame(move || bridge.borrow_mut().frame())
    }
}

/// Creates the presenter for one visible WPE page, wired to take its own
/// input.
///
/// The view answers
/// [`wants_input_events`](GpuContentView::wants_input_events), so a backend
/// that routes surface input to GPU views needs nothing WPE-specific: the
/// pointer, keyboard, scroll and composition events landing on this layer
/// reach `WPEPlatform` through
/// [`WpeSurfaceInput`](crate::WpeSurfaceInput). A backend whose input arrives
/// somewhere else entirely — GTK delivers it to the `GtkGLArea`'s event
/// controllers — builds a [`DmaBufGpuView`] and owns a `WpeSurfaceInput`
/// beside it instead.
#[cfg(feature = "webview")]
#[must_use]
pub fn gpu_view_with_input(page: WpePage) -> GpuContentView {
    WpeInputGpuView::new(
        DmaBufGpuView::new(page.clone()).into_view(),
        WpeSurfaceInput::new(page),
    )
    .into_view()
}

fn render_browser_frame(gpu: &mut GpuState, mut incoming: DmaBufFrame, frame: &Frame<'_>) {
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
    let mut import = gpu.importer.copy_into(&mut incoming, &source.texture);
    incoming.presented();
    encode_browser_blit(gpu, &bind_group, frame, &mut import.encoder);
    // The Vulkan import records its queue-family acquire/copy/release into
    // `command_buffers`; they go ahead of the blit encoder in one submission
    // (the GLES path leaves it empty and has already run its copy).
    frame.queue.submit(
        import
            .command_buffers
            .into_iter()
            .chain([import.encoder.finish()]),
    );
    let guard = import.guard;
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
    frame: &Frame<'_>,
    encoder: &mut wgpu::CommandEncoder,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("waterui_wpe_blit"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: frame.view,
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

fn create_gpu_state(context: &Context<'_>) -> GpuState {
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
                    format: context.format,
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
        target_format: context.format,
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

fn clear_target(frame: &Frame<'_>) {
    let mut encoder = frame
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("waterui_wpe_empty_encoder"),
        });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("waterui_wpe_empty"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame.view,
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
