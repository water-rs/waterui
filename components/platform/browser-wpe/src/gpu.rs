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
        let size = wgpu::Extent3d {
            width: frame.width,
            height: frame.height,
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
                imported.record_copy(&mut encoder, &destination, frame.width, frame.height);
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
        let egl_library = unsafe { libloading::Library::new("libEGL.so.1") }
            .unwrap_or_else(|error| panic!("failed to load libEGL.so.1 for WPE: {error}"));
        let gles_library = unsafe { libloading::Library::new("libGLESv2.so.2") }
            .unwrap_or_else(|error| panic!("failed to load libGLESv2.so.2 for WPE: {error}"));
        let get_proc = load_library_symbol::<EglGetProcAddress>(
            &[&egl_library],
            b"eglGetProcAddress\0",
            "eglGetProcAddress",
        );
        let egl_get_current_display = load_egl_symbol::<EglGetCurrentDisplay>(
            &egl_library,
            &gles_library,
            get_proc,
            "eglGetCurrentDisplay",
        );
        let egl_create_image = load_egl_symbol::<EglCreateImage>(
            &egl_library,
            &gles_library,
            get_proc,
            "eglCreateImageKHR",
        );
        let egl_destroy_image = load_egl_symbol::<EglDestroyImage>(
            &egl_library,
            &gles_library,
            get_proc,
            "eglDestroyImageKHR",
        );
        let image_target_texture = load_egl_symbol::<GlEglImageTargetTexture2d>(
            &egl_library,
            &gles_library,
            get_proc,
            "glEGLImageTargetTexture2DOES",
        );
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
        let display = unsafe { (self.egl_get_current_display)() };
        assert!(
            !display.is_null(),
            "WPE EGL import requires a current EGL display"
        );
        let attributes = dma_buf_egl_attributes(frame);
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
        assert_eq!(
            unsafe { (self.egl_destroy_image)(display, image) },
            1,
            "failed to destroy imported WPE EGLImage"
        );
    }

    fn blit_egl_image(&self, frame: &DmaBufFrame, destination: &wgpu::Texture, image: EglImage) {
        let width = i32::try_from(frame.width).expect("WPE frame width exceeds EGLint");
        let height = i32::try_from(frame.height).expect("WPE frame height exceeds EGLint");
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

fn load_library_symbol<T: Copy>(libraries: &[&libloading::Library], name: &[u8], label: &str) -> T {
    for library in libraries {
        if let Ok(symbol) = unsafe { library.get::<T>(name) } {
            return *symbol;
        }
    }
    panic!("required WPE GPU symbol `{label}` is unavailable")
}

fn load_egl_symbol<T: Copy>(
    egl: &libloading::Library,
    gles: &libloading::Library,
    get_proc: EglGetProcAddress,
    name: &str,
) -> T {
    let name_with_nul =
        CString::new(name).unwrap_or_else(|_| panic!("EGL symbol contains a NUL byte"));
    for library in [egl, gles] {
        if let Ok(symbol) = unsafe { library.get::<T>(name_with_nul.as_bytes_with_nul()) } {
            return *symbol;
        }
    }
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
    unsafe { std::mem::transmute_copy(&address) }
}

fn load_egl_address(
    egl: &libloading::Library,
    gles: &libloading::Library,
    get_proc: EglGetProcAddress,
    name: &str,
) -> *const c_void {
    let name_with_nul = CString::new(name).expect("GL symbol contains a NUL byte");
    for library in [egl, gles] {
        let Ok(symbol) =
            (unsafe { library.get::<*const c_void>(name_with_nul.as_bytes_with_nul()) })
        else {
            continue;
        };
        if !symbol.is_null() {
            return *symbol;
        }
    }
    unsafe { get_proc(name_with_nul.as_ptr()) }
}

struct ImportedVulkanImage {
    device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
    queue_family_index: u32,
}

unsafe impl Send for ImportedVulkanImage {}

impl ImportedVulkanImage {
    fn record_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        destination: &wgpu::Texture,
        width: u32,
        height: u32,
    ) {
        let destination = unsafe {
            destination
                .as_hal::<wgpu::hal::api::Vulkan>()
                .expect("WPE destination texture is not Vulkan")
        };
        let destination = unsafe { destination.raw_handle() };
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
        unsafe {
            self.device.destroy_image(self.image, None);
            self.device.free_memory(self.memory, None);
        }
    }
}

fn import_vulkan_dma_buf(device: &wgpu::Device, frame: &mut DmaBufFrame) -> ImportedVulkanImage {
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
    if let Err(error) = unsafe { raw.bind_image_memory(image, memory, 0) } {
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
    let requirements = unsafe { raw.get_image_memory_requirements(image) };
    let loader =
        ash::khr::external_memory_fd::Device::new(hal_device.shared_instance().raw_instance(), raw);
    let mut fd_properties = vk::MemoryFdPropertiesKHR::default();
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
    match unsafe { raw.allocate_memory(&allocation, None) } {
        Ok(memory) => memory,
        Err(error) => {
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
