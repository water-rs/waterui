use super::*;
use core::num::NonZeroU32;
#[cfg(hydrolysis_macos_system_webview)]
use objc2::rc::Retained;
#[cfg(hydrolysis_macos_system_webview)]
use objc2_web_kit::WKWebView;
use shaderloom::CompiledShader;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use shaderloom::WgslModuleCache;

const GPU_SURFACE_COMPOSITOR_SHADER: CompiledShader =
    include!(concat!(env!("OUT_DIR"), "/gpu_surface_compositor.rs"));

/// Builds a fresh `vello::Renderer` for the parallel-encode pool, matching the main
/// renderer's options (GPU-only, area AA, multi-core init).
fn build_pooled_vello_renderer(device: &wgpu::Device) -> vello::Renderer {
    vello::Renderer::new(
        device,
        vello::RendererOptions {
            use_cpu: false,
            antialiasing_support: vello::AaSupport::area_only(),
            num_init_threads: std::thread::available_parallelism().ok(),
            pipeline_cache: None,
        },
    )
    .expect("hydrolysis renderer: failed to create pooled vello renderer")
}

/// C2: encode independent Vello layers to per-layer textures across CPU cores.
///
/// Each worker checks a `vello::Renderer` out of `pool` (creating one on first use),
/// renders its scene to its own texture, and returns the texture + view tagged with the
/// originating `render_layers` index so the caller can composite in painter's order.
/// `vello::Renderer` is `!Sync`, so per-worker ownership (not sharing) is what makes this
/// sound; the GPU `Queue` is `Send + Sync` and each layer targets an independent texture,
/// so submission order is irrelevant.
fn encode_vello_layers_parallel(
    pool: &std::sync::Mutex<Vec<vello::Renderer>>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scenes: Vec<(usize, &vello::Scene, PooledLayerTexture)>,
    width: u32,
    height: u32,
) -> Vec<(usize, PooledLayerTexture)> {
    #[cfg(not(target_arch = "wasm32"))]
    use rayon::prelude::*;

    let render_layer = |(index, scene, leased): (usize, &vello::Scene, PooledLayerTexture)| {
        let mut renderer = pool
            .lock()
            .expect("hydrolysis renderer: vello renderer pool poisoned")
            .pop()
            .unwrap_or_else(|| build_pooled_vello_renderer(device));

        let params = vello::RenderParams {
            base_color: vello::peniko::Color::TRANSPARENT,
            width,
            height,
            antialiasing_method: vello::AaConfig::Area,
        };
        renderer
            .render_to_texture(device, queue, scene, &leased.view, &params)
            .expect("hydrolysis renderer: failed to render vello layer scene");

        pool.lock()
            .expect("hydrolysis renderer: vello renderer pool poisoned")
            .push(renderer);

        (index, leased)
    };

    #[cfg(not(target_arch = "wasm32"))]
    let rendered = scenes.into_par_iter().map(render_layer).collect();
    #[cfg(target_arch = "wasm32")]
    let rendered = scenes.into_iter().map(render_layer).collect();
    rendered
}

#[derive(Default)]
pub(crate) struct Compositor {
    /// Pool of target-sized intermediate textures reused across frames for
    /// per-layer Vello encodes and active-layer masks. Allocating one per
    /// layer per frame is exactly the churn the pool exists to avoid; entries
    /// whose size no longer matches the target are dropped on acquire.
    pub(crate) layer_texture_pool: Vec<PooledLayerTexture>,
    /// Pool of `vello::Renderer` instances reused across frames for C2's parallel
    /// per-layer encoding. `vello::Renderer` is `!Sync` (it holds a `RefCell`), so each
    /// worker checks out its own instance; the `Mutex` only guards the free-list, not the
    /// (parallel) encode itself.
    pub(crate) vello_renderer_pool: std::sync::Mutex<Vec<vello::Renderer>>,
    pub(crate) gpu_surface_compositor: Option<GpuSurfaceCompositorState>,
    pub(crate) render_layers: Vec<RenderLayer>,
    pub(crate) active_scene_layers: Vec<ActiveSceneLayer>,
    pub(crate) active_filter_images: Vec<vello::peniko::ImageData>,
}

pub(crate) struct PooledLayerTexture {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
}

impl Compositor {
    fn acquire_layer_texture(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> PooledLayerTexture {
        // All intermediate layer textures are target-sized, so a resize makes
        // every pooled entry stale at once; drop them instead of hoarding.
        self.layer_texture_pool
            .retain(|entry| entry.texture.width() == width && entry.texture.height() == height);
        self.layer_texture_pool.pop().unwrap_or_else(|| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("hydrolysis_layer_texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            PooledLayerTexture { texture, view }
        })
    }

    fn release_layer_texture(&mut self, texture: PooledLayerTexture) {
        self.layer_texture_pool.push(texture);
    }
}

pub(crate) struct GpuSurfaceCompositorState {
    pub(crate) target_format: wgpu::TextureFormat,
    pub(crate) uniform_buffer: wgpu::Buffer,
    /// Number of 256-byte uniform slots `uniform_buffer` holds (one per
    /// composited layer); the buffer is recreated when a frame needs more.
    pub(crate) uniform_slot_capacity: usize,
    pub(crate) sampler: wgpu::Sampler,
    pub(crate) bind_group_layout: wgpu::BindGroupLayout,
    pub(crate) pipeline: wgpu::RenderPipeline,
    pub(crate) _white_mask_texture: wgpu::Texture,
    pub(crate) white_mask_view: wgpu::TextureView,
}

pub(crate) struct EmbeddedGpuSurfaceRuntime {
    surface: Option<GpuSurface>,
    env: Option<Environment>,
    setup_complete: bool,
    msaa_samples: NonZeroU32,
    prefers_hdr: Option<bool>,
    output_format: wgpu::TextureFormat,
    output_size: (u32, u32),
    output_texture: Option<wgpu::Texture>,
    output_view: Option<wgpu::TextureView>,
    gesture: GestureState,
    trackpad_pan_ending: bool,
    redraw_handle: RedrawHandle,
    /// First frame instant, fixed when the surface first renders; the frame
    /// clock's origin for `GpuFrame::elapsed`.
    start_time: Option<Instant>,
    /// Frame instant of the previous render, for `GpuFrame::delta`.
    last_frame_time: Option<Instant>,
}

#[derive(Clone)]
struct EmbeddedGpuSurfaceSetup {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    shader_cache: Arc<WgslModuleCache>,
    scene_renderer: Arc<waterui_graphics::SharedSceneRenderer>,
    host_redraw_handle: Option<RedrawHandle>,
}

#[derive(Clone)]
pub(crate) enum LayerShape {
    Rect(vello::kurbo::Rect),
    RoundedRect {
        path: vello::kurbo::BezPath,
        #[cfg_attr(
            not(hydrolysis_macos_system_webview),
            expect(
                dead_code,
                reason = "rounded geometry is consumed by macOS native-view clipping"
            )
        )]
        rect: vello::kurbo::Rect,
        #[cfg_attr(
            not(hydrolysis_macos_system_webview),
            expect(
                dead_code,
                reason = "rounded geometry is consumed by macOS native-view clipping"
            )
        )]
        corner_width: f64,
        #[cfg_attr(
            not(hydrolysis_macos_system_webview),
            expect(
                dead_code,
                reason = "rounded geometry is consumed by macOS native-view clipping"
            )
        )]
        corner_height: f64,
    },
    Path(vello::kurbo::BezPath),
}

#[derive(Clone)]
pub(crate) struct ActiveSceneLayer {
    pub(crate) alpha: f32,
    pub(crate) transform: vello::kurbo::Affine,
    pub(crate) shape: LayerShape,
}

/// Where a [`GpuSurfaceLayer`]'s runtime lives. The retained render tree owns
/// its runtime directly inside its `GpuSurfaceNode` (`Owned`), so a reactive
/// swap renders the new surface and a per-frame re-flush re-binds the same
/// runtime structurally — no cursor desync.
#[derive(Clone)]
pub(crate) enum GpuSurfaceSource {
    Owned(Rc<RefCell<EmbeddedGpuSurfaceRuntime>>),
}

#[derive(Clone)]
pub(crate) struct GpuSurfaceLayer {
    pub(crate) source: GpuSurfaceSource,
    pub(crate) transform: vello::kurbo::Affine,
    pub(crate) bounds: vello::kurbo::Rect,
    /// The surface's rect in window hit-test space, used to project the
    /// window pointer into surface-local coordinates at composite time.
    pub(crate) hit_rect: vello::kurbo::Rect,
    pub(crate) active_layers: Vec<ActiveSceneLayer>,
    pub(crate) direct_to_target: bool,
}

#[cfg(hydrolysis_macos_system_webview)]
#[derive(Clone)]
pub(crate) struct NativeViewLayer {
    pub(crate) view: Retained<WKWebView>,
    pub(crate) transform: vello::kurbo::Affine,
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) active_layers: Vec<ActiveSceneLayer>,
    /// Where `WaterUI`-drawn interactive content covers this view, in window
    /// hit-test space, refreshed every frame by
    /// [`NativeViewOcclusion`](crate::renderer::NativeViewOcclusion). The view
    /// host refuses AppKit hits inside these rects so the content on top gets
    /// the click it visibly deserves.
    pub(crate) occlusion: Rc<RefCell<Vec<vello::kurbo::Rect>>>,
}

pub(crate) enum RenderLayer {
    Vello(vello::Scene),
    GpuSurface(GpuSurfaceLayer),
    #[cfg(hydrolysis_macos_system_webview)]
    NativeView(NativeViewLayer),
}

#[cfg(hydrolysis_macos_system_webview)]
pub(crate) struct HybridRenderSegment {
    layers: Vec<RenderLayer>,
}

#[cfg(hydrolysis_macos_system_webview)]
pub(crate) struct HybridComposition {
    pub(crate) segments: Vec<HybridRenderSegment>,
    pub(crate) native_views: Vec<NativeViewLayer>,
    pub(crate) transient_scene: Option<vello::Scene>,
}

pub(crate) struct PreparedGpuSurfaceLayer {
    pub(crate) view: wgpu::TextureView,
    pub(crate) uniform_bytes: [u8; 80],
    pub(crate) needs_redraw: bool,
}

pub(crate) fn take_gpu_surface_redraw_request(
    frame_requested_redraw: bool,
    redraw_handle: &RedrawHandle,
) -> bool {
    let external_redraw_requested = redraw_handle.take_dirty();
    frame_requested_redraw || external_redraw_requested
}

pub struct HydrolysisRenderTarget<'a> {
    pub adapter: &'a wgpu::Adapter,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub texture: Option<&'a wgpu::Texture>,
    pub view: &'a wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
    pub base_color: vello::peniko::Color,
}

pub(crate) struct DirectGpuSurfaceTarget<'a> {
    pub(crate) device: &'a wgpu::Device,
    pub(crate) queue: &'a wgpu::Queue,
    pub(crate) texture: &'a wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pointer: PointerState,
    pub(crate) now: Instant,
}

pub(crate) struct EmbeddedLayerTarget {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) transform: vello::kurbo::Affine,
    pub(crate) bounds: vello::kurbo::Rect,
    /// The surface's rect in window hit-test space; the pointer is projected
    /// into surface-local pixels inside `prepare_layer`, which is where the
    /// layer's output pixel size is decided.
    pub(crate) hit_rect: vello::kurbo::Rect,
    pub(crate) pointer_position: Option<vello::kurbo::Point>,
    pub(crate) pointer_press_origin: Option<vello::kurbo::Point>,
    pub(crate) now: Instant,
}

/// Projects the window pointer into an embedded surface's local pixel space.
///
/// `hit_rect` is the surface's rect in window hit-test coordinates and
/// `(width, height)` its output texture size. The hover position maps only
/// while inside the rect; the press origin maps only when the press started on
/// this surface, so a drag that leaves the bounds keeps reporting its origin.
pub(crate) fn project_pointer_into_surface(
    pointer_position: Option<vello::kurbo::Point>,
    pointer_press_origin: Option<vello::kurbo::Point>,
    hit_rect: vello::kurbo::Rect,
    width: u32,
    height: u32,
) -> PointerState {
    if hit_rect.width() <= 0.0 || hit_rect.height() <= 0.0 {
        return PointerState::default();
    }
    #[allow(clippy::cast_possible_truncation)]
    let map = |point: vello::kurbo::Point| {
        waterui_core::layout::Point::new(
            ((point.x - hit_rect.x0) / hit_rect.width() * f64::from(width)) as f32,
            ((point.y - hit_rect.y0) / hit_rect.height() * f64::from(height)) as f32,
        )
    };
    let position = pointer_position
        .filter(|point| hit_rect.contains(*point))
        .map(map);
    let hit = pointer_press_origin
        .filter(|origin| hit_rect.contains(*origin))
        .map(map);
    PointerState { position, hit }
}

/// One layer fully prepared for the final composite pass: its content and mask
/// views (pooled textures ride along so they return to the pool afterwards)
/// plus the 80-byte compositor uniform.
struct ReadyLayerComposite {
    layer_view: wgpu::TextureView,
    layer_texture: Option<PooledLayerTexture>,
    mask_view: wgpu::TextureView,
    mask_texture: Option<PooledLayerTexture>,
    uniform_bytes: [u8; 80],
}

impl ActiveSceneLayer {
    pub(crate) fn push_to_scene(&self, scene: &mut vello::Scene) {
        match &self.shape {
            LayerShape::Rect(rect) => {
                scene.push_layer(
                    vello::peniko::Fill::NonZero,
                    vello::peniko::BlendMode::default(),
                    self.alpha,
                    self.transform,
                    rect,
                );
            }
            LayerShape::RoundedRect { path, .. } | LayerShape::Path(path) => {
                scene.push_layer(
                    vello::peniko::Fill::NonZero,
                    vello::peniko::BlendMode::default(),
                    self.alpha,
                    self.transform,
                    path,
                );
            }
        }
    }
}

impl GpuSurfaceCompositorState {
    /// Size of one compositor uniform in bytes (the shader-visible struct).
    const UNIFORM_SIZE: u64 = 80;
    /// Stride between per-layer uniform slots: WebGPU's guaranteed
    /// `min_uniform_buffer_offset_alignment`.
    const UNIFORM_SLOT_STRIDE: u64 = 256;
    const INITIAL_UNIFORM_SLOTS: usize = 8;

    fn create_uniform_buffer(device: &wgpu::Device, slots: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_uniform"),
            size: (slots as u64) * Self::UNIFORM_SLOT_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn ensure_uniform_capacity(&mut self, device: &wgpu::Device, slots: usize) {
        if slots <= self.uniform_slot_capacity {
            return;
        }
        let slots = slots.next_power_of_two();
        self.uniform_buffer = Self::create_uniform_buffer(device, slots);
        self.uniform_slot_capacity = slots;
    }

    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let uniform_buffer = Self::create_uniform_buffer(device, Self::INITIAL_UNIFORM_SLOTS);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        // Every composited layer reads its own 256-byte-aligned
                        // slot of one shared buffer, selected per draw with a
                        // dynamic offset, so the whole composite is a single
                        // render pass and a single submit.
                        has_dynamic_offset: true,
                        min_binding_size: Some(
                            core::num::NonZeroU64::new(Self::UNIFORM_SIZE)
                                .expect("static compositor uniform size must be non-zero"),
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let (vertex_shader, fragment_shader) =
            GPU_SURFACE_COMPOSITOR_SHADER.create_render_stages(device, "vs_main", "fs_main");
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: vertex_shader.module(),
                entry_point: Some(vertex_shader.entry_point()),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: fragment_shader.module(),
                entry_point: Some(fragment_shader.entry_point()),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let white_mask_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_white_mask"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            white_mask_texture.as_image_copy(),
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let white_mask_view =
            white_mask_texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            target_format,
            uniform_buffer,
            uniform_slot_capacity: Self::INITIAL_UNIFORM_SLOTS,
            sampler,
            bind_group_layout,
            pipeline,
            _white_mask_texture: white_mask_texture,
            white_mask_view,
        }
    }

    pub(crate) fn ensure_target_format(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) {
        if self.target_format == target_format {
            return;
        }
        *self = Self::new(device, queue, target_format);
    }
}

impl EmbeddedGpuSurfaceRuntime {
    pub(crate) fn new(surface: GpuSurface, env: &Environment) -> Self {
        let msaa_samples = surface.msaa_sample_limit();
        let prefers_hdr = surface.resolved_hdr_preference().or_else(|| {
            env.get::<DynamicRangePreference>()
                .map(|preference| preference.0)
        });
        Self {
            surface: Some(surface),
            env: Some(env.clone()),
            setup_complete: false,
            msaa_samples,
            prefers_hdr,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            output_size: (1, 1),
            output_texture: None,
            output_view: None,
            gesture: GestureState::new(),
            trackpad_pan_ending: false,
            redraw_handle: RedrawHandle::new(),
            start_time: None,
            last_frame_time: None,
        }
    }

    /// Advances the surface's animation clock to the renderer's frame instant
    /// and returns `(elapsed, delta)` for this frame. Driving the clock from
    /// the frame instant (not wall time) keeps offscreen hosts that pump the
    /// clock deterministic.
    fn frame_timing(&mut self, now: Instant) -> (Duration, Duration) {
        let start = *self.start_time.get_or_insert(now);
        let elapsed = now.saturating_duration_since(start);
        let delta = self.last_frame_time.map_or_else(
            || Duration::from_secs_f32(1.0 / 60.0),
            |last| {
                now.saturating_duration_since(last)
                    .min(Duration::from_millis(100))
            },
        );
        self.last_frame_time = Some(now);
        (elapsed, delta)
    }

    pub(crate) fn take_external_redraw_request(&self) -> bool {
        self.redraw_handle.take_dirty()
    }

    pub(crate) fn handle_trackpad_pan(&mut self, dx: f32, dy: f32, phase: TouchPhase) -> bool {
        match phase {
            TouchPhase::Started => {
                self.gesture.pan_offset = waterui_core::layout::Point::new(dx, dy);
                self.gesture.active = true;
                self.trackpad_pan_ending = false;
            }
            TouchPhase::Moved => {
                if !self.gesture.active {
                    self.gesture.pan_offset = waterui_core::layout::Point::zero();
                    self.gesture.active = true;
                }
                self.trackpad_pan_ending = false;
                self.gesture.pan_offset.x += dx;
                self.gesture.pan_offset.y += dy;
            }
            TouchPhase::Ended => {
                if !self.gesture.active {
                    self.gesture.pan_offset = waterui_core::layout::Point::zero();
                    self.gesture.active = true;
                }
                self.gesture.pan_offset.x += dx;
                self.gesture.pan_offset.y += dy;
                self.trackpad_pan_ending = true;
            }
            TouchPhase::Cancelled => {
                self.trackpad_pan_ending = self.gesture.active;
            }
        }
        self.redraw_handle.request_redraw();
        true
    }

    fn finish_trackpad_pan_frame(&mut self) {
        if self.trackpad_pan_ending {
            self.trackpad_pan_ending = false;
            self.gesture.active = false;
            self.redraw_handle.request_redraw();
        }
    }

    pub(crate) fn prepare_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: EmbeddedLayerTarget,
    ) -> PreparedGpuSurfaceLayer {
        let top_left =
            target.transform * vello::kurbo::Point::new(target.bounds.x0, target.bounds.y0);
        let top_right =
            target.transform * vello::kurbo::Point::new(target.bounds.x1, target.bounds.y0);
        let bottom_right =
            target.transform * vello::kurbo::Point::new(target.bounds.x1, target.bounds.y1);
        let bottom_left =
            target.transform * vello::kurbo::Point::new(target.bounds.x0, target.bounds.y1);

        let layer_width =
            edge_length_in_pixels(top_left, top_right, target.width, target.height).max(1);
        let layer_height =
            edge_length_in_pixels(top_left, bottom_left, target.width, target.height).max(1);
        let output_format = self.output_format;
        self.ensure_output_target(device, layer_width, layer_height, output_format);

        let (elapsed, delta) = self.frame_timing(target.now);
        let texture = self
            .output_texture
            .as_ref()
            .expect("hydrolysis embedded GpuSurface missing output texture");
        let view = self
            .output_view
            .as_ref()
            .expect("hydrolysis embedded GpuSurface missing output view")
            .clone();
        let pointer = project_pointer_into_surface(
            target.pointer_position,
            target.pointer_press_origin,
            target.hit_rect,
            layer_width,
            layer_height,
        );
        let mut frame = GpuFrame::new(
            device,
            queue,
            texture,
            view.clone(),
            output_format,
            layer_width,
            layer_height,
            pointer,
            self.gesture,
            elapsed,
            delta,
        );
        assert!(
            self.setup_complete,
            "hydrolysis embedded GpuSurface used before setup"
        );
        self.surface
            .as_mut()
            .expect("hydrolysis embedded GpuSurface missing after setup")
            .render(&mut frame);
        let frame_requested_redraw = frame.was_redraw_requested();
        drop(frame);
        self.finish_trackpad_pan_frame();
        let needs_redraw =
            take_gpu_surface_redraw_request(frame_requested_redraw, &self.redraw_handle);
        let corners = [
            point_to_clip(top_left, target.width, target.height),
            point_to_clip(top_right, target.width, target.height),
            point_to_clip(bottom_right, target.width, target.height),
            point_to_clip(bottom_left, target.width, target.height),
        ];

        PreparedGpuSurfaceLayer {
            view,
            uniform_bytes: encode_compositor_uniform(corners, false),
            needs_redraw,
        }
    }

    pub(crate) fn render_direct_to_target(&mut self, target: DirectGpuSurfaceTarget<'_>) -> bool {
        let (elapsed, delta) = self.frame_timing(target.now);
        let mut frame = GpuFrame::new(
            target.device,
            target.queue,
            target.texture,
            target.view,
            target.format,
            target.width,
            target.height,
            target.pointer,
            self.gesture,
            elapsed,
            delta,
        );
        assert!(
            self.setup_complete,
            "hydrolysis embedded GpuSurface used before setup"
        );
        self.surface
            .as_mut()
            .expect("hydrolysis embedded GpuSurface missing after setup")
            .render(&mut frame);
        let frame_requested_redraw = frame.was_redraw_requested();
        drop(frame);
        self.finish_trackpad_pan_frame();
        take_gpu_surface_redraw_request(frame_requested_redraw, &self.redraw_handle)
    }

    async fn setup(
        runtime: Rc<RefCell<Self>>,
        resources: EmbeddedGpuSurfaceSetup,
        surface_format: wgpu::TextureFormat,
    ) {
        let (surface, env, msaa_samples, redraw_handle) = {
            let mut runtime = runtime.borrow_mut();
            if runtime.setup_complete && runtime.output_format == surface_format {
                return;
            }
            let surface = runtime
                .surface
                .take()
                .expect("hydrolysis embedded GpuSurface setup started concurrently");
            let env = runtime
                .env
                .take()
                .expect("hydrolysis embedded GpuSurface environment missing before setup");
            runtime.setup_complete = false;
            (
                surface,
                env,
                runtime.msaa_samples,
                runtime.redraw_handle.clone(),
            )
        };

        let wake_parent: Option<Arc<dyn Fn() + Send + Sync>> =
            resources.host_redraw_handle.as_ref().map(|handle| {
                let handle = handle.clone();
                Arc::new(move || handle.request_redraw()) as Arc<dyn Fn() + Send + Sync>
            });
        redraw_handle.set_waker(wake_parent);

        let mut surface = surface;
        let mut env = env;
        {
            // `GpuContext::new` resolves the surface's declared MSAA limit
            // against what the adapter actually supports for this format, so
            // the renderer sees a sample count it can really use rather than
            // the authoring-side cap.
            let context = GpuContext::new(
                &resources.adapter,
                &resources.device,
                &resources.queue,
                surface_format,
                resources.shader_cache.as_ref(),
                &resources.scene_renderer,
                msaa_samples.get(),
                redraw_handle,
            );
            surface.setup(&context, &mut env).await;
        }
        let mut runtime = runtime.borrow_mut();
        runtime.surface = Some(surface);
        runtime.env = Some(env);
        runtime.output_format = surface_format;
        runtime.setup_complete = true;
    }

    fn ensure_setup(
        runtime: &Rc<RefCell<Self>>,
        resources: EmbeddedGpuSurfaceSetup,
        signals: FrameSignals,
        surface_format: wgpu::TextureFormat,
    ) -> bool {
        {
            let runtime = runtime.borrow();
            if runtime.setup_complete && runtime.output_format == surface_format {
                return true;
            }
            if runtime.surface.is_none() {
                return false;
            }
        }

        let wake_host = resources.host_redraw_handle.clone();
        let runtime = Rc::clone(runtime);
        spawn_local(async move {
            Self::setup(runtime, resources, surface_format).await;
            signals.request_redraw();
            if let Some(handle) = wake_host {
                handle.request_redraw();
            }
        })
        .detach();
        false
    }

    pub(crate) fn output_format_for(
        &self,
        target_format: wgpu::TextureFormat,
    ) -> wgpu::TextureFormat {
        select_embedded_surface_format(target_format, self.prefers_hdr)
    }

    fn ensure_output_target(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) {
        let needs_recreate = self.output_texture.is_none()
            || self.output_size != (width, height)
            || self.output_format != format;
        if !needs_recreate {
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_embedded_gpu_surface_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.output_format = format;
        self.output_size = (width, height);
        self.output_texture = Some(texture);
        self.output_view = Some(view);
    }
}

fn select_embedded_surface_format(
    target_format: wgpu::TextureFormat,
    prefers_hdr_override: Option<bool>,
) -> wgpu::TextureFormat {
    let target_hdr = matches!(
        target_format,
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    );
    let prefers_hdr = prefers_hdr_override.unwrap_or(true);
    if target_hdr && prefers_hdr {
        return wgpu::TextureFormat::Rgba16Float;
    }
    wgpu::TextureFormat::Rgba8Unorm
}

fn point_to_clip(point: vello::kurbo::Point, width: u32, height: u32) -> [f32; 2] {
    assert!(
        width != 0 && height != 0,
        "hydrolysis compositor target size must be non-zero"
    );

    let clip_x = ((point.x as f32) / (width as f32)) * 2.0 - 1.0;
    let clip_y = 1.0 - ((point.y as f32) / (height as f32)) * 2.0;
    [clip_x, clip_y]
}

fn edge_length_in_pixels(
    start: vello::kurbo::Point,
    end: vello::kurbo::Point,
    target_width: u32,
    target_height: u32,
) -> u32 {
    assert!(
        target_width != 0 && target_height != 0,
        "hydrolysis compositor target size must be non-zero"
    );
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    ((dx * dx + dy * dy).sqrt().round().max(1.0)) as u32
}

fn encode_compositor_uniform(corners: [[f32; 2]; 4], source_is_srgb: bool) -> [u8; 80] {
    let uvs = [[0.0f32, 0.0f32], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let mut bytes = [0u8; 80];
    for (index, corner) in corners.iter().enumerate() {
        let base = index * 16;
        write_f32(&mut bytes, base, corner[0]);
        write_f32(&mut bytes, base + 4, corner[1]);
        write_f32(&mut bytes, base + 8, uvs[index][0]);
        write_f32(&mut bytes, base + 12, uvs[index][1]);
    }
    write_f32(&mut bytes, 64, if source_is_srgb { 1.0 } else { 0.0 });
    bytes
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

impl HydrolysisRenderer {
    #[cfg(hydrolysis_macos_system_webview)]
    pub(crate) fn take_hybrid_composition(&mut self) -> Option<HybridComposition> {
        self.flush_vello_scene_layer();
        if !self
            .compositor
            .render_layers
            .iter()
            .any(|layer| matches!(layer, RenderLayer::NativeView(_)))
        {
            return None;
        }

        let mut segments = vec![HybridRenderSegment { layers: Vec::new() }];
        let mut native_views = Vec::new();
        for layer in core::mem::take(&mut self.compositor.render_layers) {
            match layer {
                RenderLayer::NativeView(layer) => {
                    native_views.push(layer);
                    segments.push(HybridRenderSegment { layers: Vec::new() });
                }
                layer => segments
                    .last_mut()
                    .expect("Hydrolysis hybrid composition must have a render segment")
                    .layers
                    .push(layer),
            }
        }
        assert!(
            segments.len() == native_views.len() + 1,
            "Hydrolysis hybrid composition segment count must bracket every native view"
        );
        Some(HybridComposition {
            segments,
            native_views,
            transient_scene: self.transient_scene.take(),
        })
    }

    #[cfg(hydrolysis_macos_system_webview)]
    pub(crate) fn render_hybrid_segment_to_surface(
        &mut self,
        segment: &mut HybridRenderSegment,
        transient_scene: Option<vello::Scene>,
        target: HydrolysisRenderTarget<'_>,
    ) {
        assert!(
            self.compositor.render_layers.is_empty(),
            "Hydrolysis hybrid composition cannot render over retained layers"
        );
        assert!(
            self.transient_scene.is_none(),
            "Hydrolysis hybrid composition cannot replace a transient scene"
        );
        self.compositor.render_layers = core::mem::take(&mut segment.layers);
        self.transient_scene = transient_scene;
        self.render_scene_to_surface(target);
        segment.layers = core::mem::take(&mut self.compositor.render_layers);
        assert!(
            self.transient_scene.is_none(),
            "Hydrolysis hybrid segment left a transient scene unconsumed"
        );
    }

    #[cfg(hydrolysis_macos_system_webview)]
    pub(crate) fn restore_hybrid_composition(&mut self, composition: HybridComposition) {
        let HybridComposition {
            segments,
            native_views,
            transient_scene,
        } = composition;
        assert!(
            transient_scene.is_none(),
            "Hydrolysis hybrid composition restored before rendering its transient scene"
        );
        assert!(
            segments.len() == native_views.len() + 1,
            "Hydrolysis hybrid composition segment count changed during rendering"
        );
        let segment_count = segments.len();
        let mut native_views = native_views.into_iter();
        let mut layers = Vec::new();
        for (index, segment) in segments.into_iter().enumerate() {
            layers.extend(segment.layers);
            if index + 1 < segment_count
                && let Some(native_view) = native_views.next()
            {
                layers.push(RenderLayer::NativeView(native_view));
            }
        }
        assert!(
            native_views.next().is_none(),
            "Hydrolysis hybrid composition did not restore every native view"
        );
        self.compositor.render_layers = layers;
    }

    fn embedded_gpu_surface_setup(
        &self,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> EmbeddedGpuSurfaceSetup {
        EmbeddedGpuSurfaceSetup {
            adapter: adapter.clone(),
            device: device.clone(),
            queue: queue.clone(),
            shader_cache: Arc::clone(&self.shader_cache),
            scene_renderer: Arc::clone(&self.scene_renderer),
            host_redraw_handle: self.host_redraw_handle.clone(),
        }
    }

    /// Await setup for every GPU surface reachable from the statically built
    /// retained tree. A `HydrolysisGpuView` calls this from its own async setup,
    /// making its first sized frame fully ready without polling or fixed retries.
    pub(crate) async fn setup_embedded_gpu_surfaces(&self, context: &GpuContext<'_>) {
        let runtimes = self.node_gpu_surfaces.clone();
        for runtime in runtimes {
            let surface_format = runtime.borrow().output_format_for(context.surface_format);
            EmbeddedGpuSurfaceRuntime::setup(
                runtime,
                self.embedded_gpu_surface_setup(context.adapter, context.device, context.queue),
                surface_format,
            )
            .await;
        }
    }

    pub fn render_scene_to_texture(&mut self, target: HydrolysisRenderTarget<'_>) {
        self.render_scene_to_surface(target);
    }

    fn ensure_gpu_surface_compositor_state(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) {
        if self.compositor.gpu_surface_compositor.is_none() {
            self.compositor.gpu_surface_compositor =
                Some(GpuSurfaceCompositorState::new(device, queue, target_format));
            return;
        }
        self.compositor
            .gpu_surface_compositor
            .as_mut()
            .expect("hydrolysis renderer: missing gpu surface compositor state")
            .ensure_target_format(device, queue, target_format);
    }

    /// Renders a scene into a pooled target-sized texture, which the caller
    /// must hand back to the pool once the composite pass has sampled it.
    fn render_vello_layer_to_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &vello::Scene,
        width: u32,
        height: u32,
    ) -> PooledLayerTexture {
        let leased = self.compositor.acquire_layer_texture(device, width, height);
        let params = vello::RenderParams {
            base_color: vello::peniko::Color::TRANSPARENT,
            width,
            height,
            antialiasing_method: vello::AaConfig::Area,
        };
        self.vello_renderer
            .render_to_texture(device, queue, scene, &leased.view, &params)
            .expect("hydrolysis renderer: failed to render vello layer scene");
        leased
    }

    fn render_active_layers_mask_to_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        active_layers: &[ActiveSceneLayer],
    ) -> PooledLayerTexture {
        assert!(
            !active_layers.is_empty(),
            "hydrolysis renderer: active layer mask requires at least one layer"
        );
        let mut mask_scene = vello::Scene::new();
        for layer in active_layers {
            layer.push_to_scene(&mut mask_scene);
        }
        mask_scene.fill(
            vello::peniko::Fill::NonZero,
            vello::kurbo::Affine::IDENTITY,
            vello::peniko::Color::WHITE,
            None,
            &vello::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
        );
        for _ in 0..active_layers.len() {
            mask_scene.pop_layer();
        }
        self.render_vello_layer_to_texture(device, queue, &mask_scene, width, height)
    }

    fn default_compositor_mask_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> wgpu::TextureView {
        self.ensure_gpu_surface_compositor_state(device, queue, target_format);
        self.compositor
            .gpu_surface_compositor
            .as_ref()
            .expect("hydrolysis renderer: missing gpu surface compositor state")
            .white_mask_view
            .clone()
    }

    fn clear_target_surface(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        base_color: vello::peniko::Color,
    ) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hydrolysis_surface_clear_encoder"),
        });
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hydrolysis_surface_clear_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color_to_wgpu(base_color)),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        drop(_pass);
        queue.submit(std::iter::once(encoder.finish()));
    }

    /// Composites every prepared layer into the target in painter's order with
    /// one render pass and one submit. Each layer's uniform lives in its own
    /// dynamic-offset slot of the shared buffer, so nothing forces a
    /// submit-per-layer round trip.
    fn composite_ready_layers(
        &mut self,
        target: &HydrolysisRenderTarget<'_>,
        layers: &[ReadyLayerComposite],
    ) {
        self.ensure_gpu_surface_compositor_state(target.device, target.queue, target.format);
        let compositor = self
            .compositor
            .gpu_surface_compositor
            .as_mut()
            .expect("hydrolysis renderer: missing gpu surface compositor state");
        compositor.ensure_uniform_capacity(target.device, layers.len());

        let stride = GpuSurfaceCompositorState::UNIFORM_SLOT_STRIDE as usize;
        let mut uniform_bytes = vec![0u8; layers.len() * stride];
        for (index, layer) in layers.iter().enumerate() {
            let start = index * stride;
            uniform_bytes[start..start + layer.uniform_bytes.len()]
                .copy_from_slice(&layer.uniform_bytes);
        }
        target
            .queue
            .write_buffer(&compositor.uniform_buffer, 0, &uniform_bytes);

        let bind_groups: Vec<wgpu::BindGroup> = layers
            .iter()
            .map(|layer| {
                target.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("hydrolysis_gpu_surface_compositor_bind_group"),
                    layout: &compositor.bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &compositor.uniform_buffer,
                                offset: 0,
                                size: core::num::NonZeroU64::new(
                                    GpuSurfaceCompositorState::UNIFORM_SIZE,
                                ),
                            }),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&compositor.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&layer.layer_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(&layer.mask_view),
                        },
                    ],
                })
            })
            .collect();

        let mut encoder = target
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hydrolysis_gpu_surface_compositor_encoder"),
            });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color_to_wgpu(target.base_color)),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&compositor.pipeline);
        for (index, bind_group) in bind_groups.iter().enumerate() {
            let offset = (index as u64) * GpuSurfaceCompositorState::UNIFORM_SLOT_STRIDE;
            #[allow(clippy::cast_possible_truncation)]
            pass.set_bind_group(0, bind_group, &[offset as u32]);
            pass.draw(0..6, 0..1);
        }
        drop(pass);
        target.queue.submit(std::iter::once(encoder.finish()));
    }

    pub fn render_scene_to_surface(&mut self, target: HydrolysisRenderTarget<'_>) {
        assert!(
            matches!(
                target.format.remove_srgb_suffix(),
                wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
            ) || matches!(
                target.format,
                wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
            ),
            "hydrolysis renderer: unsupported surface format {:?}",
            target.format
        );

        self.flush_vello_scene_layer();
        let fullscreen_uniform =
            encode_compositor_uniform([[-1.0, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]], true);
        let mut render_layers = core::mem::take(&mut self.compositor.render_layers);
        let transient_layer_count =
            if let Some(scene) = self.transient_scene.take().filter(scene_has_content) {
                render_layers.push(RenderLayer::Vello(scene));
                1
            } else {
                0
            };
        if render_layers.is_empty() {
            self.clear_target_surface(target.device, target.queue, target.view, target.base_color);
            return;
        }
        if let [RenderLayer::GpuSurface(layer)] = render_layers.as_slice()
            && layer.direct_to_target
        {
            let texture = target
                .texture
                .expect("hydrolysis direct GpuSurface render requires target texture");
            let direct_target = DirectGpuSurfaceTarget {
                device: target.device,
                queue: target.queue,
                texture,
                view: target.view.clone(),
                format: target.format,
                width: target.width,
                height: target.height,
                pointer: project_pointer_into_surface(
                    self.hit_test.pointer_position,
                    self.hit_test.pointer_press_origin,
                    layer.hit_rect,
                    target.width,
                    target.height,
                ),
                now: self.frame_instant(),
            };
            let GpuSurfaceSource::Owned(runtime) = &layer.source;
            if !EmbeddedGpuSurfaceRuntime::ensure_setup(
                runtime,
                self.embedded_gpu_surface_setup(target.adapter, target.device, target.queue),
                self.frame_signals(),
                target.format,
            ) {
                self.clear_target_surface(
                    target.device,
                    target.queue,
                    target.view,
                    target.base_color,
                );
                self.compositor.render_layers = render_layers;
                return;
            }
            let needs_redraw = runtime.borrow_mut().render_direct_to_target(direct_target);
            self.compositor.render_layers = render_layers;
            if needs_redraw {
                self.request_redraw();
            }
            return;
        }
        let mut needs_redraw = false;

        // Phase 1 — prepare every layer's content and mask textures. All the
        // per-layer content work (parallel Vello encodes, embedded GpuSurface
        // renders, mask rasterization) lands here, before any composite state
        // is touched.
        //
        // C2: when there are 2+ independent Vello layers, encode them to
        // per-layer pooled textures across CPU cores up front; the loop below
        // consumes the results in painter's order. A single Vello layer keeps
        // the sequential path (nothing to parallelize), and GpuSurface layers
        // are unaffected.
        let mut encoded_vello: Vec<Option<PooledLayerTexture>> =
            (0..render_layers.len()).map(|_| None).collect();
        {
            let vello_indices: Vec<usize> = render_layers
                .iter()
                .enumerate()
                .filter_map(|(index, layer)| match layer {
                    RenderLayer::Vello(_) => Some(index),
                    RenderLayer::GpuSurface(_) => None,
                    #[cfg(hydrolysis_macos_system_webview)]
                    RenderLayer::NativeView(_) => {
                        panic!("Hydrolysis native views require hybrid window composition")
                    }
                })
                .collect();
            if vello_indices.len() > 1 {
                let vello_scenes: Vec<(usize, &vello::Scene, PooledLayerTexture)> = vello_indices
                    .iter()
                    .map(|&index| {
                        let leased = self.compositor.acquire_layer_texture(
                            target.device,
                            target.width,
                            target.height,
                        );
                        let RenderLayer::Vello(scene) = &render_layers[index] else {
                            panic!("hydrolysis renderer: vello layer index changed type");
                        };
                        (index, scene, leased)
                    })
                    .collect();
                for (index, leased) in encode_vello_layers_parallel(
                    &self.compositor.vello_renderer_pool,
                    target.device,
                    target.queue,
                    vello_scenes,
                    target.width,
                    target.height,
                ) {
                    encoded_vello[index] = Some(leased);
                }
            }
        }

        let mut ready: Vec<ReadyLayerComposite> = Vec::with_capacity(render_layers.len());
        for (layer_index, layer) in render_layers.iter().enumerate() {
            match layer {
                RenderLayer::Vello(scene) => {
                    tracing::trace!(
                        layer_index,
                        paths = scene.encoding().n_paths,
                        segments = scene.encoding().n_path_segments,
                        "compositing Hydrolysis Vello layer"
                    );
                    let leased = match encoded_vello[layer_index].take() {
                        Some(leased) => leased,
                        None => self.render_vello_layer_to_texture(
                            target.device,
                            target.queue,
                            scene,
                            target.width,
                            target.height,
                        ),
                    };
                    let mask_view = self.default_compositor_mask_view(
                        target.device,
                        target.queue,
                        target.format,
                    );
                    ready.push(ReadyLayerComposite {
                        layer_view: leased.view.clone(),
                        layer_texture: Some(leased),
                        mask_view,
                        mask_texture: None,
                        uniform_bytes: fullscreen_uniform,
                    });
                }
                // Embedded GPU surfaces render serially by design: `GpuView` is a
                // user-facing, main-thread contract (`!Send` setup/render futures,
                // `&mut Environment`), so parallelizing this loop would force
                // `Send` onto every user renderer. Vello layers get their
                // parallelism in `encode_vello_layers_parallel` instead.
                RenderLayer::GpuSurface(layer) => {
                    tracing::trace!(
                        layer_index,
                        bounds = ?layer.bounds,
                        transform = ?layer.transform,
                        "compositing Hydrolysis GPU surface layer"
                    );
                    let embedded_target = EmbeddedLayerTarget {
                        width: target.width,
                        height: target.height,
                        transform: layer.transform,
                        bounds: layer.bounds,
                        hit_rect: layer.hit_rect,
                        pointer_position: self.hit_test.pointer_position,
                        pointer_press_origin: self.hit_test.pointer_press_origin,
                        now: self.frame_instant(),
                    };
                    let GpuSurfaceSource::Owned(runtime) = &layer.source;
                    let output_format = runtime.borrow().output_format_for(target.format);
                    if !EmbeddedGpuSurfaceRuntime::ensure_setup(
                        runtime,
                        self.embedded_gpu_surface_setup(
                            target.adapter,
                            target.device,
                            target.queue,
                        ),
                        self.frame_signals(),
                        output_format,
                    ) {
                        needs_redraw = true;
                        continue;
                    }
                    let prepared = runtime.borrow_mut().prepare_layer(
                        target.device,
                        target.queue,
                        embedded_target,
                    );
                    if prepared.needs_redraw {
                        needs_redraw = true;
                    }
                    let (mask_view, mask_texture) = if layer.active_layers.is_empty() {
                        (
                            self.default_compositor_mask_view(
                                target.device,
                                target.queue,
                                target.format,
                            ),
                            None,
                        )
                    } else {
                        let leased = self.render_active_layers_mask_to_texture(
                            target.device,
                            target.queue,
                            target.width,
                            target.height,
                            &layer.active_layers,
                        );
                        (leased.view.clone(), Some(leased))
                    };
                    ready.push(ReadyLayerComposite {
                        layer_view: prepared.view,
                        layer_texture: None,
                        mask_view,
                        mask_texture,
                        uniform_bytes: prepared.uniform_bytes,
                    });
                }
                #[cfg(hydrolysis_macos_system_webview)]
                RenderLayer::NativeView(_) => {
                    panic!("Hydrolysis native views require hybrid window composition")
                }
            }
        }

        // Phase 2 — one render pass, one submit, painter's order.
        if ready.is_empty() {
            self.clear_target_surface(target.device, target.queue, target.view, target.base_color);
        } else {
            self.composite_ready_layers(&target, &ready);
        }
        for layer in ready {
            if let Some(leased) = layer.layer_texture {
                self.compositor.release_layer_texture(leased);
            }
            if let Some(leased) = layer.mask_texture {
                self.compositor.release_layer_texture(leased);
            }
        }
        for _ in 0..transient_layer_count {
            render_layers.pop();
        }
        self.compositor.render_layers = render_layers;

        if needs_redraw {
            self.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct GestureProbe;

    impl waterui_graphics::GpuView for GestureProbe {
        async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut Environment) {}

        fn render(&mut self, _frame: &mut GpuFrame) {}
    }

    #[test]
    fn trackpad_pan_reaches_one_active_frame_before_settling() {
        let mut runtime =
            EmbeddedGpuSurfaceRuntime::new(GpuSurface::new(GestureProbe), &Environment::new());

        assert!(runtime.handle_trackpad_pan(3.0, -2.0, TouchPhase::Started));
        assert!(runtime.handle_trackpad_pan(24.0, -12.0, TouchPhase::Moved));
        assert_eq!(
            runtime.gesture.pan_offset,
            waterui_core::layout::Point::new(27.0, -14.0)
        );
        assert!(runtime.gesture.active);

        assert!(runtime.handle_trackpad_pan(2.0, -1.0, TouchPhase::Ended));
        assert_eq!(
            runtime.gesture.pan_offset,
            waterui_core::layout::Point::new(29.0, -15.0)
        );
        assert!(runtime.gesture.active);
        assert!(runtime.trackpad_pan_ending);

        runtime.finish_trackpad_pan_frame();
        assert!(!runtime.gesture.active);
        assert!(!runtime.trackpad_pan_ending);
    }

    #[test]
    fn embedded_surface_inherits_dynamic_range_metadata() {
        let mut env = Environment::new();
        env.insert(DynamicRangePreference(false));
        let runtime = EmbeddedGpuSurfaceRuntime::new(GpuSurface::new(GestureProbe), &env);
        assert_eq!(runtime.prefers_hdr, Some(false));

        let explicit = EmbeddedGpuSurfaceRuntime::new(
            GpuSurface::new(GestureProbe).prefer_hdr_surface(),
            &env,
        );
        assert_eq!(explicit.prefers_hdr, Some(true));
    }
}
