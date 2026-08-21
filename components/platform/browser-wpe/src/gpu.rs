use ash::vk;
use glow::HasContext as _;
use num_traits::ToPrimitive as _;
use std::cell::Cell;
use std::ffi::{CString, c_char, c_int, c_uint, c_void};
use std::os::fd::{AsRawFd, IntoRawFd};
use std::rc::Rc;
use waterui_graphics::gpu_surface::{GpuContext, GpuFrame, GpuView};

#[cfg(feature = "webview")]
use crate::WpePage;
use crate::{DmaBufFormat, DmaBufFrame};

struct SourceTexture {
    size: (u32, u32),
    format: wgpu::TextureFormat,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct GpuState {
    backend: GpuBackend,
    target_format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    options: wgpu::Buffer,
    source: Option<SourceTexture>,
}

enum GpuBackend {
    Vulkan,
    Gles(Box<GlesInterop>),
}

/// Copies a borrowed browser DMA-BUF into an application-owned GPU texture.
///
/// This copier is intended for browser callbacks whose native buffer becomes
/// invalid when the callback returns. The copy is completed on the GPU before
/// [`Self::copy`] returns; pixels are never read back to the CPU.
pub struct DmaBufFrameCopier {
    device: wgpu::Device,
    queue: wgpu::Queue,
    backend: GpuBackend,
}

impl core::fmt::Debug for DmaBufFrameCopier {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DmaBufFrameCopier")
            .finish_non_exhaustive()
    }
}

impl DmaBufFrameCopier {
    /// Creates a copier for the active `WaterUI` GPU backend.
    ///
    /// # Panics
    ///
    /// Panics unless the `WaterUI` device uses Vulkan or EGL/GLES.
    #[must_use]
    pub fn new(context: &GpuContext<'_>) -> Self {
        Self {
            device: context.device.clone(),
            queue: context.queue.clone(),
            backend: create_gpu_backend(context.adapter.get_info().backend),
        }
    }

    /// Copies `frame` into a texture owned exclusively by the caller.
    ///
    /// # Panics
    ///
    /// Panics when the frame's rendering fence has not signalled or GPU import,
    /// copying, or synchronization fails.
    #[must_use]
    pub fn copy(&self, mut frame: DmaBufFrame) -> wgpu::Texture {
        assert!(
            frame.is_render_ready(),
            "browser DMA-BUF must be ready before a synchronous GPU copy"
        );
        // The destination is the *visible* extent: a browser's shared image may
        // be allocated with alignment padding beyond it, and copying the padded
        // buffer then presenting it edge to edge stretched the picture and drew
        // the gutter. The source import still uses the buffer's own dimensions
        // and stride, so this only narrows what is taken from it.
        let (visible_width, visible_height) = frame.visible_size();
        let size = wgpu::Extent3d {
            width: visible_width,
            height: visible_height,
            depth_or_array_layers: 1,
        };
        let destination = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("waterui_browser_owned_dma_buf_frame"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: frame.format.texture_format(),
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        match &self.backend {
            GpuBackend::Vulkan => {
                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("waterui_browser_owned_dma_buf_copy"),
                        });
                encoder.clear_texture(&destination, &wgpu::ImageSubresourceRange::default());
                let imported = import_vulkan_dma_buf(&self.device, &mut frame);
                imported.record_copy(&mut encoder, &destination, visible_width, visible_height);
                frame.lease.presented();
                let submission = self.queue.submit([encoder.finish()]);
                self.device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submission),
                        timeout: None,
                    })
                    .expect("browser DMA-BUF Vulkan copy failed");
                drop(imported);
            }
            GpuBackend::Gles(gles) => {
                gles.copy_dma_buf(&frame, &destination);
                // SAFETY: every glow entry point is unsafe because it requires
                // the GL context its function pointers were loaded from to be
                // current on this thread. `GlesInterop` is neither `Send` nor
                // `Sync` (it holds the `libloading` handles and raw EGL
                // pointers) and is reached here only through `&self` on the UI
                // thread, which is where wgpu's GLES device keeps its context
                // current. `glFinish` takes no arguments, so currency is the
                // only precondition; it is what makes the copy above complete
                // before `presented()` lets the browser reuse the buffer.
                unsafe {
                    gles.gl.finish();
                }
                frame.lease.presented();
            }
        }
        frame.lease.release(None);
        destination
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

/// GPU view that composites a Linux browser DMA-BUF stream without CPU readback.
pub struct DmaBufGpuView<S> {
    source: S,
    viewport: WpeViewport,
    gpu: Option<GpuState>,
    pending_frame: Option<DmaBufFrame>,
}

/// WPE-specialized DMA-BUF GPU view.
#[cfg(feature = "webview")]
pub type WpeGpuView = DmaBufGpuView<WpePage>;

/// Shared viewport scale updated by the host backend.
#[derive(Debug, Clone)]
pub struct WpeViewport(Rc<Cell<f64>>);

impl WpeViewport {
    /// Creates a viewport at device scale 1.
    #[must_use]
    pub fn new() -> Self {
        Self(Rc::new(Cell::new(1.0)))
    }

    /// Creates a viewport from a host-owned shared scale cell.
    #[doc(hidden)]
    #[must_use]
    pub const fn from_shared(scale: Rc<Cell<f64>>) -> Self {
        Self(scale)
    }

    /// Updates the device scale used for WPE layout and input coordinates.
    ///
    /// # Panics
    ///
    /// Panics when `scale` is not positive and finite.
    pub fn set_scale(&self, scale: f64) {
        assert!(
            scale.is_finite() && scale > 0.0,
            "WPE viewport scale must be positive and finite"
        );
        self.0.set(scale);
    }

    /// Returns the current device scale.
    #[must_use]
    pub fn scale(&self) -> f64 {
        self.0.get()
    }
}

impl Default for WpeViewport {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> core::fmt::Debug for DmaBufGpuView<S> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("WpeGpuView").finish_non_exhaustive()
    }
}

impl<S: DmaBufFrameSource> DmaBufGpuView<S> {
    /// Creates a renderer for `source`.
    #[must_use]
    pub fn new(source: S) -> Self {
        Self::with_viewport(source, WpeViewport::new())
    }

    /// Creates a renderer driven by a backend-owned viewport scale.
    #[must_use]
    pub const fn with_viewport(source: S, viewport: WpeViewport) -> Self {
        Self {
            source,
            viewport,
            gpu: None,
            pending_frame: None,
        }
    }

    /// Creates a renderer whose host backend forwards all input directly.
    #[must_use]
    pub const fn with_external_input(source: S, viewport: WpeViewport) -> Self {
        Self::with_viewport(source, viewport)
    }

    /// Returns the frame source.
    #[must_use]
    pub const fn source(&self) -> &S {
        &self.source
    }
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
        resize_browser_source(&self.source, &self.viewport, frame);
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

fn resize_browser_source<S: DmaBufFrameSource>(
    source: &S,
    viewport: &WpeViewport,
    frame: &GpuFrame<'_>,
) {
    let scale = viewport.scale();
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
    let (mut encoder, imported) = copy_browser_source(gpu, &mut incoming, frame);
    incoming.lease.presented();
    encode_browser_blit(gpu, &bind_group, frame, &mut encoder);
    frame.queue.submit([encoder.finish()]);
    frame.queue.on_submitted_work_done(move || {
        drop(imported);
        incoming.lease.release(None);
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

fn copy_browser_source(
    gpu: &GpuState,
    incoming: &mut DmaBufFrame,
    frame: &GpuFrame<'_>,
) -> (wgpu::CommandEncoder, Option<ImportedVulkanImage>) {
    let source = gpu
        .source
        .as_ref()
        .expect("WPE source texture must exist before import");
    let mut encoder = frame
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("waterui_wpe_encoder"),
        });
    encoder.clear_texture(&source.texture, &wgpu::ImageSubresourceRange::default());
    let imported = match &gpu.backend {
        GpuBackend::Vulkan => {
            let imported = import_vulkan_dma_buf(frame.device, incoming);
            imported.record_copy(
                &mut encoder,
                &source.texture,
                incoming.width,
                incoming.height,
            );
            Some(imported)
        }
        GpuBackend::Gles(gles) => {
            frame.queue.submit([encoder.finish()]);
            gles.copy_dma_buf(incoming, &source.texture);
            encoder = frame
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("waterui_wpe_gles_blit_encoder"),
                });
            None
        }
    };
    (encoder, imported)
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
    let backend = create_gpu_backend(context.adapter.get_info().backend);
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
        backend,
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

fn create_gpu_backend(backend: wgpu::Backend) -> GpuBackend {
    match backend {
        wgpu::Backend::Vulkan => GpuBackend::Vulkan,
        wgpu::Backend::Gl => GpuBackend::Gles(Box::new(GlesInterop::new())),
        backend => {
            panic!("bundled WPE requires Vulkan or EGL/GLES GPU import, received {backend:?}")
        }
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

type EglDisplay = *mut c_void;
type EglImage = *mut c_void;
type EglGetCurrentDisplay = unsafe extern "C" fn() -> EglDisplay;
type EglCreateImage =
    unsafe extern "C" fn(EglDisplay, *mut c_void, c_uint, *mut c_void, *const c_int) -> EglImage;
type EglDestroyImage = unsafe extern "C" fn(EglDisplay, EglImage) -> c_uint;
type EglGetProcAddress = unsafe extern "C" fn(*const c_char) -> *const c_void;
type GlEglImageTargetTexture2d = unsafe extern "C" fn(c_uint, EglImage);

const EGL_NONE: c_int = 0x3038;
const EGL_WIDTH: c_int = 0x3057;
const EGL_HEIGHT: c_int = 0x3056;
const EGL_LINUX_DMA_BUF_EXT: c_uint = 0x3270;
const EGL_LINUX_DRM_FOURCC_EXT: c_int = 0x3271;
const EGL_DMA_BUF_PLANE0_FD_EXT: c_int = 0x3272;
const EGL_DMA_BUF_PLANE0_OFFSET_EXT: c_int = 0x3273;
const EGL_DMA_BUF_PLANE0_PITCH_EXT: c_int = 0x3274;
const EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT: c_int = 0x3443;
const EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT: c_int = 0x3444;
const DRM_FORMAT_MOD_INVALID: u64 = u64::MAX;

struct GlesInterop {
    gl: glow::Context,
    egl_get_current_display: EglGetCurrentDisplay,
    egl_create_image: EglCreateImage,
    egl_destroy_image: EglDestroyImage,
    image_target_texture: GlEglImageTargetTexture2d,
    _egl_library: libloading::Library,
    _gles_library: libloading::Library,
}

impl GlesInterop {
    fn new() -> Self {
        // SAFETY: `Library::new` is unsafe because `dlopen` runs the library's
        // initializers, which can execute arbitrary code. These are the two
        // system EGL/GLES runtime libraries named by their versioned SONAMEs,
        // not a caller-supplied path, and the process is already running on
        // them: wgpu's GLES backend opened the same libraries to create the
        // device this interop serves, so `dlopen` returns the existing handle
        // and no new initializer runs. Both handles are stored in `Self`, so
        // every symbol resolved below stays valid for as long as it is callable.
        let egl_library = unsafe { libloading::Library::new("libEGL.so.1") }
            .unwrap_or_else(|error| panic!("failed to load libEGL.so.1 for WPE: {error}"));
        // SAFETY: as above, for the GLES runtime.
        let gles_library = unsafe { libloading::Library::new("libGLESv2.so.2") }
            .unwrap_or_else(|error| panic!("failed to load libGLESv2.so.2 for WPE: {error}"));
        // SAFETY: `EglGetProcAddress` is the signature of `eglGetProcAddress`,
        // which is the name being resolved, satisfying `load_library_symbol`'s
        // contract. A platform that does not export it panics inside the loader
        // rather than returning a null pointer to call through.
        let get_proc = unsafe {
            load_library_symbol::<EglGetProcAddress>(
                &[&egl_library],
                b"eglGetProcAddress\0",
                "eglGetProcAddress",
            )
        };
        // SAFETY: `EglGetCurrentDisplay` is the signature of
        // `eglGetCurrentDisplay`, the name resolved here, which is what
        // `load_egl_symbol` requires; an absent name panics in the loader.
        let egl_get_current_display = unsafe {
            load_egl_symbol::<EglGetCurrentDisplay>(
                &egl_library,
                &gles_library,
                get_proc,
                "eglGetCurrentDisplay",
            )
        };
        // SAFETY: `EglCreateImage` is the signature of `eglCreateImageKHR`, the
        // name resolved here; an absent name panics in the loader.
        let egl_create_image = unsafe {
            load_egl_symbol::<EglCreateImage>(
                &egl_library,
                &gles_library,
                get_proc,
                "eglCreateImageKHR",
            )
        };
        // SAFETY: `EglDestroyImage` is the signature of `eglDestroyImageKHR`,
        // the name resolved here; an absent name panics in the loader.
        let egl_destroy_image = unsafe {
            load_egl_symbol::<EglDestroyImage>(
                &egl_library,
                &gles_library,
                get_proc,
                "eglDestroyImageKHR",
            )
        };
        // SAFETY: `GlEglImageTargetTexture2d` is the signature of
        // `glEGLImageTargetTexture2DOES`, the name resolved here. This one is an
        // extension entry point, so it usually arrives through
        // `eglGetProcAddress` rather than `dlsym`; either way the loader panics
        // if the driver lacks it.
        let image_target_texture = unsafe {
            load_egl_symbol::<GlEglImageTargetTexture2d>(
                &egl_library,
                &gles_library,
                get_proc,
                "glEGLImageTargetTexture2DOES",
            )
        };
        // SAFETY: `from_loader_function` requires the loader to return either a
        // null pointer or a pointer to a function with the signature glow
        // expects for that name. `load_egl_address` resolves names only through
        // `dlsym` on the two GL libraries above and `eglGetProcAddress`, so any
        // non-null result is the platform's own implementation of exactly that
        // GL entry point; unknown names come back null, which glow records as
        // unavailable rather than calling. glow copies the pointers out during
        // this call, and the libraries backing them are kept alive by the
        // handles moved into `Self` below.
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                load_egl_address(&egl_library, &gles_library, get_proc, name)
            })
        };
        Self {
            gl,
            egl_get_current_display,
            egl_create_image,
            egl_destroy_image,
            image_target_texture,
            _egl_library: egl_library,
            _gles_library: gles_library,
        }
    }

    fn copy_dma_buf(&self, frame: &DmaBufFrame, destination: &wgpu::Texture) {
        // SAFETY: `egl_get_current_display` holds the address `dlsym`/
        // `eglGetProcAddress` returned for `eglGetCurrentDisplay`, whose EGL
        // signature is exactly the `EglGetCurrentDisplay` alias. It takes no
        // arguments and only reads the calling thread's EGL binding, so the
        // sole precondition is being on the thread wgpu made current — the UI
        // thread, which `&self` on this non-`Send` type guarantees. A thread
        // with no current context yields `EGL_NO_DISPLAY`, caught just below.
        let display = unsafe { (self.egl_get_current_display)() };
        assert!(
            !display.is_null(),
            "WPE EGL import requires a current EGL display"
        );
        let attributes = dma_buf_egl_attributes(frame);
        // SAFETY: `egl_create_image` is the resolved `eglCreateImageKHR`, whose
        // signature matches `EglCreateImage`. `display` was just asserted
        // non-null and comes from this thread's current binding; the context
        // and buffer arguments are `EGL_NO_CONTEXT`/`NULL`, which is what the
        // `EGL_LINUX_DMA_BUF_EXT` target requires. `attributes` is built by
        // `dma_buf_egl_attributes`, which terminates the list with `EGL_NONE`
        // and passes the plane's file descriptor while `frame` still owns it,
        // so the descriptor is open for the whole call. EGL does not take
        // ownership of that descriptor, so `frame` may still close it later.
        // `attributes` outlives the call, as EGL only reads it here.
        let image = unsafe {
            (self.egl_create_image)(
                display,
                std::ptr::null_mut(),
                EGL_LINUX_DMA_BUF_EXT,
                std::ptr::null_mut(),
                attributes.as_ptr(),
            )
        };
        assert!(!image.is_null(), "failed to import WPE DMA-BUF as EGLImage");
        self.blit_egl_image(frame, destination, image);
        // SAFETY: the resolved `eglDestroyImageKHR`, matching `EglDestroyImage`.
        // `image` was created non-null from `display` just above and has not
        // been destroyed since; `blit_egl_image` only binds it to a texture and
        // does not consume it. Destroying it here is the single matching
        // release for that single creation, and the GL work referencing it has
        // already been recorded against the texture.
        let destroyed = unsafe { (self.egl_destroy_image)(display, image) };
        assert_eq!(destroyed, 1, "failed to destroy imported WPE EGLImage");
    }

    fn blit_egl_image(&self, frame: &DmaBufFrame, destination: &wgpu::Texture, image: EglImage) {
        // The visible extent, which is the buffer's own size unless a browser
        // padded its shared image; the destination texture was sized to match.
        let (visible_width, visible_height) = frame.visible_size();
        let width = i32::try_from(visible_width).expect("WPE frame width exceeds EGLint");
        let height = i32::try_from(visible_height).expect("WPE frame height exceeds EGLint");
        // SAFETY: `Texture::as_hal` is unsafe because it exposes the backend
        // object behind wgpu's tracking, so the caller must both name the
        // backend the texture really belongs to and not invalidate wgpu's view
        // of it. The backend is checked: this method is only reached from
        // `copy_dma_buf`, which `create_gpu_backend` selects solely for
        // `wgpu::Backend::Gl`, and a mismatch surfaces as `None` and panics
        // here rather than being transmuted. The guard's only use is to read
        // `inner` for the raw texture name; the texture is not destroyed,
        // reallocated, or relabelled, and the blit below writes through the
        // ordinary GL pipeline, which is a state wgpu re-establishes for its
        // own next command.
        let destination = unsafe {
            destination
                .as_hal::<wgpu::hal::api::Gles>()
                .expect("WPE destination texture is not GLES")
        };
        let wgpu::hal::gles::TextureInner::Texture {
            raw: destination_texture,
            target: destination_target,
        } = destination.inner
        else {
            panic!("WPE destination must be a GLES texture");
        };
        assert_eq!(
            destination_target,
            glow::TEXTURE_2D,
            "WPE destination must be a two-dimensional GLES texture"
        );

        // SAFETY: every call in this block is a glow GL entry point, unsafe for
        // the one shared reason that GL requires its context to be current on
        // the calling thread; `&self` on this non-`Send` type reaches here only
        // on the UI thread, where wgpu keeps the GLES context current. Beyond
        // currency the arguments are checked rather than assumed:
        // `source_texture`, `read` and `draw` are names GL just handed back,
        // each used only between its creation and its deletion at the end of
        // the block; `destination_texture`/`destination_target` come from the
        // live hal guard above and `destination_target` was asserted to be
        // `TEXTURE_2D`; `image` is the non-null `EGLImage` its caller keeps
        // alive across this call; and both framebuffers are asserted complete
        // before `blit_framebuffer` reads or writes through them, with `width`
        // and `height` converted from the frame's own dimensions. The block
        // unbinds both framebuffers and the texture before deleting them, so it
        // leaves no name bound for wgpu's next command to trip over.
        unsafe {
            let source_texture = self
                .gl
                .create_texture()
                .unwrap_or_else(|error| panic!("failed to create WPE GLES source: {error}"));
            self.gl.bind_texture(glow::TEXTURE_2D, Some(source_texture));
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                i32::try_from(glow::NEAREST).expect("GL_NEAREST must fit GLint"),
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                i32::try_from(glow::NEAREST).expect("GL_NEAREST must fit GLint"),
            );
            (self.image_target_texture)(glow::TEXTURE_2D, image);

            let read = self
                .gl
                .create_framebuffer()
                .unwrap_or_else(|error| panic!("failed to create WPE GLES read FBO: {error}"));
            let draw = self
                .gl
                .create_framebuffer()
                .unwrap_or_else(|error| panic!("failed to create WPE GLES draw FBO: {error}"));
            self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(read));
            self.gl.framebuffer_texture_2d(
                glow::READ_FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(source_texture),
                0,
            );
            assert_eq!(
                self.gl.check_framebuffer_status(glow::READ_FRAMEBUFFER),
                glow::FRAMEBUFFER_COMPLETE,
                "WPE EGLImage read framebuffer is incomplete"
            );
            self.gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(draw));
            self.gl.framebuffer_texture_2d(
                glow::DRAW_FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                destination_target,
                Some(destination_texture),
                0,
            );
            assert_eq!(
                self.gl.check_framebuffer_status(glow::DRAW_FRAMEBUFFER),
                glow::FRAMEBUFFER_COMPLETE,
                "WPE destination framebuffer is incomplete"
            );
            self.gl.blit_framebuffer(
                0,
                0,
                width,
                height,
                0,
                0,
                width,
                height,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            self.gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            self.gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
            self.gl.bind_texture(glow::TEXTURE_2D, None);
            self.gl.delete_framebuffer(read);
            self.gl.delete_framebuffer(draw);
            self.gl.delete_texture(source_texture);
        }
    }
}

fn dma_buf_egl_attributes(frame: &DmaBufFrame) -> Vec<c_int> {
    assert_eq!(
        frame.planes.len(),
        1,
        "WPE EGL import requires one packed DMA-BUF plane"
    );
    let plane = &frame.planes[0];
    let fourcc = match frame.format {
        DmaBufFormat::Bgra8 => u32::from_le_bytes(*b"AR24"),
        DmaBufFormat::Bgrx8 => u32::from_le_bytes(*b"XR24"),
        DmaBufFormat::Rgba8 => u32::from_le_bytes(*b"AB24"),
        DmaBufFormat::Rgbx8 => u32::from_le_bytes(*b"XB24"),
    };
    let mut attributes = vec![
        EGL_WIDTH,
        i32::try_from(frame.width).expect("WPE frame width exceeds EGLint"),
        EGL_HEIGHT,
        i32::try_from(frame.height).expect("WPE frame height exceeds EGLint"),
        EGL_LINUX_DRM_FOURCC_EXT,
        i32::from_ne_bytes(fourcc.to_ne_bytes()),
        EGL_DMA_BUF_PLANE0_FD_EXT,
        plane.fd.as_raw_fd(),
        EGL_DMA_BUF_PLANE0_OFFSET_EXT,
        i32::try_from(plane.offset).expect("WPE plane offset exceeds EGLint"),
        EGL_DMA_BUF_PLANE0_PITCH_EXT,
        i32::try_from(plane.stride).expect("WPE plane stride exceeds EGLint"),
    ];
    if frame.modifier != DRM_FORMAT_MOD_INVALID {
        let modifier_low = u32::try_from(frame.modifier & u64::from(u32::MAX))
            .expect("DMA-BUF modifier low bits must fit u32");
        let modifier_high =
            u32::try_from(frame.modifier >> 32).expect("DMA-BUF modifier high bits must fit u32");
        attributes.extend([
            EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT,
            i32::from_ne_bytes(modifier_low.to_ne_bytes()),
            EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT,
            i32::from_ne_bytes(modifier_high.to_ne_bytes()),
        ]);
    }
    attributes.push(EGL_NONE);
    attributes
}

/// Resolves `name` from the first library that exports it.
///
/// # Safety
///
/// `T` must be the type of the symbol named by `name` in whichever of
/// `libraries` exports it. Callers in this module pass the `unsafe extern "C"`
/// aliases declared above, each matching the EGL/GL signature for the name it
/// is paired with.
unsafe fn load_library_symbol<T: Copy>(
    libraries: &[&libloading::Library],
    name: &[u8],
    label: &str,
) -> T {
    for library in libraries {
        // SAFETY: `Library::get` is unsafe because it reinterprets the address
        // `dlsym` returns as `T` without being able to check it. The caller
        // guarantees, per this function's own contract, that `T` is the type of
        // `name` in these libraries. `name` is a NUL-terminated byte string, as
        // `dlsym` requires. The returned `Symbol` borrows the library, and the
        // `*symbol` copy is a bare function pointer whose validity is tied to
        // the library staying loaded — which `GlesInterop` ensures by owning
        // both handles for as long as it can call them.
        if let Ok(symbol) = unsafe { library.get::<T>(name) } {
            return *symbol;
        }
    }
    panic!("required WPE GPU symbol `{label}` is unavailable")
}

/// Resolves an EGL/GL entry point, falling back to `eglGetProcAddress` for the
/// extension entry points the libraries do not export directly.
///
/// # Safety
///
/// `T` must be the type of the symbol named `name`. Every caller in this module
/// pairs one of the `unsafe extern "C"` aliases declared above with the EGL name
/// it was written for.
unsafe fn load_egl_symbol<T: Copy>(
    egl: &libloading::Library,
    gles: &libloading::Library,
    get_proc: EglGetProcAddress,
    name: &str,
) -> T {
    let name_with_nul =
        CString::new(name).unwrap_or_else(|_| panic!("EGL symbol contains a NUL byte"));
    for library in [egl, gles] {
        // SAFETY: `T` is the symbol's type by this function's contract, and
        // `as_bytes_with_nul` supplies the NUL-terminated name `dlsym` wants.
        // The copied-out function pointer stays valid because `GlesInterop`
        // keeps both libraries loaded for as long as it holds the pointer.
        if let Ok(symbol) = unsafe { library.get::<T>(name_with_nul.as_bytes_with_nul()) } {
            return *symbol;
        }
    }
    // SAFETY: `get_proc` is the address resolved for `eglGetProcAddress`, whose
    // signature is `EglGetProcAddress`. It takes a NUL-terminated name, which
    // `as_ptr` provides from a `CString` that outlives the call, and returns
    // either null or the entry point for that name.
    let address = unsafe { get_proc(name_with_nul.as_ptr()) };
    assert!(
        !address.is_null(),
        "required WPE GPU symbol `{name}` is unavailable"
    );
    assert_eq!(
        size_of::<T>(),
        size_of::<*const c_void>(),
        "WPE GPU symbol pointer has an unexpected size"
    );
    // SAFETY: `transmute_copy` reinterprets the resolved address as `T`. The
    // sizes are asserted equal immediately above, so no memory outside
    // `address` is read; `T` is one of the `unsafe extern "C" fn` aliases, whose
    // validity invariant is being a non-null pointer to a function of that
    // signature — non-null is asserted, and the signature holds by this
    // function's contract. The address belongs to a library `GlesInterop` keeps
    // loaded, so the resulting pointer stays callable.
    unsafe { std::mem::transmute_copy(&address) }
}

/// Resolves the address of GL entry point `name`, or null when it is absent.
///
/// This is glow's loader, so it deliberately returns a bare address rather than
/// a typed pointer: glow is what knows the signature for each name.
fn load_egl_address(
    egl: &libloading::Library,
    gles: &libloading::Library,
    get_proc: EglGetProcAddress,
    name: &str,
) -> *const c_void {
    let name_with_nul = CString::new(name).expect("GL symbol contains a NUL byte");
    for library in [egl, gles] {
        // SAFETY: `Library::get` needs the requested type to describe the
        // symbol. The type asked for here is `*const c_void`, the address
        // itself, which is what `dlsym` returns for any symbol whatsoever, so
        // no signature is being claimed and no call is made through it. The
        // name is NUL-terminated as `dlsym` requires.
        let Ok(symbol) =
            (unsafe { library.get::<*const c_void>(name_with_nul.as_bytes_with_nul()) })
        else {
            continue;
        };
        if !symbol.is_null() {
            return *symbol;
        }
    }
    // SAFETY: `get_proc` is the resolved `eglGetProcAddress`, matching
    // `EglGetProcAddress`. Its one argument must be a NUL-terminated name,
    // which `as_ptr` gives from a `CString` alive for the whole call; it
    // returns null for names the driver does not implement. Extension entry
    // points such as `glEGLImageTargetTexture2DOES` are reachable only this
    // way, which is why the `dlsym` attempts above are allowed to miss.
    unsafe { get_proc(name_with_nul.as_ptr()) }
}

struct ImportedVulkanImage {
    device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
    queue_family_index: u32,
}

// SAFETY: `ash::Device` is not `Send` only because it wraps the dispatchable
// `VkDevice` handle as a raw pointer; the image and memory fields are
// non-dispatchable `u64` handles. Vulkan permits a `VkDevice` to be used from
// any thread, and the two calls this type makes off-thread —
// `vkDestroyImage` and `vkFreeMemory` in `Drop` — require external
// synchronization only on the objects they destroy. This type owns its image
// and memory outright: they are created in `import_vulkan_dma_buf`, never
// handed out or cloned, and destroyed exactly once here, so no other thread can
// name them. `Send` is what lets `render_browser_frame` move the import into
// wgpu's `on_submitted_work_done` callback, which is where it must be dropped:
// that is the point at which the GPU has finished the copy that reads the
// image.
unsafe impl Send for ImportedVulkanImage {}

impl ImportedVulkanImage {
    fn record_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        destination: &wgpu::Texture,
        width: u32,
        height: u32,
    ) {
        // SAFETY: `Texture::as_hal` requires naming the backend the texture
        // actually has and leaving wgpu's own view of it intact. The backend is
        // checked rather than assumed — `create_gpu_backend` selects
        // `GpuBackend::Vulkan` only for `wgpu::Backend::Vulkan`, and a mismatch
        // yields `None` and panics here. The guard is used only to read the
        // handle, and it is still alive at that point because it is bound to a
        // local.
        let destination = unsafe {
            destination
                .as_hal::<wgpu::hal::api::Vulkan>()
                .expect("WPE destination texture is not Vulkan")
        };
        // SAFETY: `raw_handle` exposes the underlying `VkImage`, valid for as
        // long as the wgpu texture it came from lives. The caller holds that
        // texture by reference across this whole function, and the handle is
        // only recorded into a command buffer that is submitted before the
        // borrow ends.
        let destination = unsafe { destination.raw_handle() };
        // SAFETY: recording raw Vulkan into a wgpu encoder. `as_hal_mut` needs
        // the right backend, which is checked as above and panics on `None`,
        // and requires that the commands recorded leave the encoder in a state
        // wgpu can keep using. This block records only pipeline barriers and one
        // `vkCmdCopyImage`; it starts and ends no render pass and allocates no
        // resources, so the encoder is exactly where wgpu left it afterwards.
        // The two barriers form a matched pair that acquires `self.image` from
        // `QUEUE_FAMILY_EXTERNAL` into this device's queue family and releases
        // it back, which is what the DMA-BUF's external ownership requires; the
        // layouts they name match the ones `vkCmdCopyImage` is given. The
        // destination is transitioned by wgpu itself, which knows it as a
        // `COPY_DST` texture. Extents come from the frame the image was imported
        // at, so the copy stays inside both images.
        unsafe {
            encoder.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|encoder| {
                let encoder = encoder.expect("WPE command encoder is not Vulkan");
                let subresource_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1);
                let acquire = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .old_layout(vk::ImageLayout::GENERAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
                    .dst_queue_family_index(self.queue_family_index)
                    .image(self.image)
                    .subresource_range(subresource_range);
                self.device.cmd_pipeline_barrier(
                    encoder.raw_handle(),
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[acquire],
                );
                let region = vk::ImageCopy::default()
                    .src_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(0)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .dst_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(0)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .extent(vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    });
                self.device.cmd_copy_image(
                    encoder.raw_handle(),
                    self.image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    destination,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
                let release = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
                    .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .src_queue_family_index(self.queue_family_index)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
                    .image(self.image)
                    .subresource_range(subresource_range);
                self.device.cmd_pipeline_barrier(
                    encoder.raw_handle(),
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[release],
                );
            });
        }
    }
}

impl Drop for ImportedVulkanImage {
    fn drop(&mut self) {
        // SAFETY: both handles were created in `import_vulkan_dma_buf`, are
        // owned solely by this value, and are destroyed exactly once here. The
        // image is destroyed before the memory it is bound to, as Vulkan
        // requires. Neither may still be in use by the GPU: both paths that
        // create an `ImportedVulkanImage` guarantee this before dropping it —
        // `DmaBufFrameCopier::copy` waits on the submission with
        // `PollType::Wait`, and `render_browser_frame` defers the drop into
        // `on_submitted_work_done`. Freeing the memory also closes the DMA-BUF
        // descriptor Vulkan took ownership of at import.
        unsafe {
            self.device.destroy_image(self.image, None);
            self.device.free_memory(self.memory, None);
        }
    }
}

fn import_vulkan_dma_buf(device: &wgpu::Device, frame: &mut DmaBufFrame) -> ImportedVulkanImage {
    // SAFETY: `Device::as_hal` requires the named backend to be the device's
    // real one and that the exposed device is not used to invalidate wgpu's
    // state. The backend is checked: this function is reached only through
    // `GpuBackend::Vulkan`, which `create_gpu_backend` selects for
    // `wgpu::Backend::Vulkan` alone, and a mismatch panics here instead of being
    // reinterpreted. The raw device is used only to create a new image and
    // memory of this function's own, never to touch anything wgpu owns.
    let hal_device = unsafe {
        device
            .as_hal::<wgpu::hal::api::Vulkan>()
            .expect("bundled WPE requires a Vulkan device")
    };
    validate_vulkan_import(&hal_device, frame.modifier);
    let raw = hal_device.raw_device();
    let queue_family_index = hal_device.queue_family_index();
    let plane = frame
        .planes
        .pop()
        .expect("WPE packed DMA-BUF frame must contain one plane");
    let image = create_vulkan_import_image(raw, frame, &plane);
    let memory = import_vulkan_image_memory(&hal_device, image, plane);
    // SAFETY: `image` and `memory` were both just created on `raw`, so they
    // belong to this device and neither has been bound before — this is the one
    // and only bind for each. `validate_vulkan_import` established that the
    // device enables the external-memory and DRM-modifier extensions the pair
    // was created with. The memory was allocated from a type in
    // `vkGetImageMemoryRequirements(image).memoryTypeBits` (intersected with
    // what the descriptor supports) and sized to that requirement's `size`,
    // with a `VkMemoryDedicatedAllocateInfo` naming this exact image, so
    // offset 0 satisfies the alignment requirement by construction.
    if let Err(error) = unsafe { raw.bind_image_memory(image, memory, 0) } {
        // SAFETY: binding failed, so nothing owns these yet and neither is in
        // use by the GPU. Both are still live handles from `raw`, destroyed
        // exactly once here, memory after the image bound to it. Freeing the
        // memory closes the imported DMA-BUF descriptor Vulkan took over.
        unsafe {
            raw.free_memory(memory, None);
            raw.destroy_image(image, None);
        }
        panic!("failed to bind WPE DMA-BUF Vulkan image memory: {error}");
    }
    ImportedVulkanImage {
        device: raw.clone(),
        image,
        memory,
        queue_family_index,
    }
}

fn validate_vulkan_import(hal_device: &wgpu::hal::vulkan::Device, modifier: u64) {
    assert!(
        hal_device
            .enabled_device_extensions()
            .contains(&ash::khr::external_memory_fd::NAME),
        "Vulkan device does not enable VK_KHR_external_memory_fd"
    );
    assert!(
        hal_device
            .enabled_device_extensions()
            .contains(&ash::ext::external_memory_dma_buf::NAME),
        "Vulkan device does not enable VK_EXT_external_memory_dma_buf"
    );
    assert!(
        hal_device
            .enabled_device_extensions()
            .contains(&ash::ext::image_drm_format_modifier::NAME),
        "Vulkan device does not enable VK_EXT_image_drm_format_modifier"
    );
    assert_ne!(
        modifier, DRM_FORMAT_MOD_INVALID,
        "Vulkan DMA-BUF import requires an explicit DRM format modifier"
    );
}

fn create_vulkan_import_image(
    raw: &ash::Device,
    frame: &DmaBufFrame,
    plane: &crate::DmaBufPlane,
) -> vk::Image {
    let handle_type = vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT;
    let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(handle_type);
    let plane_layout = vk::SubresourceLayout {
        offset: u64::from(plane.offset),
        size: 0,
        row_pitch: u64::from(plane.stride),
        array_pitch: 0,
        depth_pitch: 0,
    };
    let mut modifier = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
        .drm_format_modifier(frame.modifier)
        .plane_layouts(std::slice::from_ref(&plane_layout));
    let create_info = vk::ImageCreateInfo::default()
        .push_next(&mut external)
        .push_next(&mut modifier)
        .image_type(vk::ImageType::TYPE_2D)
        .format(match frame.format {
            DmaBufFormat::Bgra8 | DmaBufFormat::Bgrx8 => vk::Format::B8G8R8A8_UNORM,
            DmaBufFormat::Rgba8 | DmaBufFormat::Rgbx8 => vk::Format::R8G8B8A8_UNORM,
        })
        .extent(vk::Extent3D {
            width: frame.width,
            height: frame.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(vk::ImageUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    // SAFETY: `create_info` is fully initialized above and its two `push_next`
    // extension structs — `VkExternalMemoryImageCreateInfo` and
    // `VkImageDrmFormatModifierExplicitCreateInfoEXT` — are local `mut`
    // bindings that outlive this call, as the borrow checker enforces through
    // `ImageCreateInfo`'s lifetime parameter. `plane_layout` likewise outlives
    // the borrow `plane_layouts` takes of it. The extensions those structs
    // require were asserted enabled by `validate_vulkan_import`, which also
    // rejected `DRM_FORMAT_MOD_INVALID`, so `DRM_FORMAT_MODIFIER_EXT` tiling
    // has the explicit modifier it demands and the single plane layout matches
    // the single-plane packed format asserted at the EGL/DMA-BUF boundary.
    unsafe { raw.create_image(&create_info, None) }
        .unwrap_or_else(|error| panic!("failed to create Vulkan DMA-BUF import image: {error}"))
}

fn import_vulkan_image_memory(
    hal_device: &wgpu::hal::vulkan::Device,
    image: vk::Image,
    plane: crate::DmaBufPlane,
) -> vk::DeviceMemory {
    let raw = hal_device.raw_device();
    let handle_type = vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT;
    // SAFETY: `image` was created on `raw` by the caller and has not been
    // destroyed, which is all `vkGetImageMemoryRequirements` requires; it only
    // reads the image and writes the returned struct.
    let requirements = unsafe { raw.get_image_memory_requirements(image) };
    let loader =
        ash::khr::external_memory_fd::Device::new(hal_device.shared_instance().raw_instance(), raw);
    let mut fd_properties = vk::MemoryFdPropertiesKHR::default();
    // SAFETY: `VK_KHR_external_memory_fd` was asserted enabled by
    // `validate_vulkan_import`, so the loader's entry point exists. The
    // descriptor is borrowed from `plane`, which still owns it here, so it is
    // open for the call; the query does not consume it. `fd_properties` is an
    // initialized local the call writes through. On failure the image created
    // by the caller is destroyed before panicking — it is not yet bound to any
    // memory and nothing else refers to it.
    unsafe {
        loader
            .get_memory_fd_properties(handle_type, plane.fd.as_raw_fd(), &mut fd_properties)
            .unwrap_or_else(|error| {
                raw.destroy_image(image, None);
                panic!("failed to query WPE DMA-BUF Vulkan memory properties: {error}")
            });
    }
    let type_bits = requirements.memory_type_bits & fd_properties.memory_type_bits;
    assert!(
        type_bits != 0,
        "WPE DMA-BUF is incompatible with every Vulkan memory type"
    );
    // SAFETY: the instance and physical device both come from the live hal
    // device guard the caller holds, so they are valid and belong together.
    // The query only reads them and returns a value.
    let memory_properties = unsafe {
        hal_device
            .shared_instance()
            .raw_instance()
            .get_physical_device_memory_properties(hal_device.raw_physical_device())
    };
    let memory_type_index = select_memory_type(type_bits, &memory_properties);
    let imported_fd = plane.fd.into_raw_fd();
    let mut import = vk::ImportMemoryFdInfoKHR::default()
        .handle_type(handle_type)
        .fd(imported_fd);
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let allocation = vk::MemoryAllocateInfo::default()
        .push_next(&mut import)
        .push_next(&mut dedicated)
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    // SAFETY: `allocation` is fully initialized and its two `push_next` structs
    // are local `mut` bindings outliving the call. `VK_KHR_external_memory_fd`
    // and `VK_EXT_external_memory_dma_buf` were asserted enabled by
    // `validate_vulkan_import`, so `DMA_BUF_EXT` is an accepted handle type.
    // `memory_type_index` was chosen from `type_bits`, the intersection of the
    // image's requirements with the types the descriptor supports, which was
    // asserted non-empty. `imported_fd` is open and, per the Vulkan spec, is
    // transferred to the implementation on success — which is why nothing
    // closes it afterwards and why `plane.fd` was consumed with `into_raw_fd`
    // rather than borrowed.
    match unsafe { raw.allocate_memory(&allocation, None) } {
        Ok(memory) => memory,
        Err(error) => {
            // SAFETY: on failure the implementation did *not* take the
            // descriptor, so this side still owns it and must close it exactly
            // once; `imported_fd` has not been closed and no `OwnedFd` holds it
            // any more, `into_raw_fd` having released it. The image is a live
            // handle from `raw`, unbound and unused by the GPU, destroyed once.
            unsafe {
                libc::close(imported_fd);
                raw.destroy_image(image, None);
            }
            panic!("failed to import WPE DMA-BUF Vulkan memory: {error}");
        }
    }
}

fn select_memory_type(type_bits: u32, properties: &vk::PhysicalDeviceMemoryProperties) -> u32 {
    let mut first = None;
    for index in 0..properties.memory_type_count {
        if type_bits & (1 << index) == 0 {
            continue;
        }
        first.get_or_insert(index);
        let memory_type = properties.memory_types
            [usize::try_from(index).expect("Vulkan memory index must fit usize")];
        if memory_type
            .property_flags
            .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        {
            return index;
        }
    }
    first.expect("WPE compatible Vulkan memory type disappeared")
}
