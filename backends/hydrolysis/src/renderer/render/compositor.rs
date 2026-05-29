use super::*;

const GPU_SURFACE_COMPOSITOR_SHADER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/shaders/gpu_surface_compositor.wgsl"
));

#[derive(Default)]
pub(crate) struct Compositor {
    pub(crate) vello_layer_surface: Option<VelloLayerSurfaceState>,
    pub(crate) gpu_surface_compositor: Option<GpuSurfaceCompositorState>,
    pub(crate) gpu_surface_slots: Vec<EmbeddedGpuSurfaceRuntime>,
    pub(crate) gpu_surface_cursor: usize,
    pub(crate) render_layers: Vec<RenderLayer>,
    pub(crate) active_scene_layers: Vec<ActiveSceneLayer>,
    pub(crate) active_filter_images: Vec<vello::peniko::ImageData>,
}

pub(crate) struct VelloLayerSurfaceState {
    pub(crate) size: (u32, u32),
    pub(crate) _texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
}

pub(crate) struct GpuSurfaceCompositorState {
    pub(crate) target_format: wgpu::TextureFormat,
    pub(crate) uniform_buffer: wgpu::Buffer,
    pub(crate) sampler: wgpu::Sampler,
    pub(crate) bind_group_layout: wgpu::BindGroupLayout,
    pub(crate) pipeline: wgpu::RenderPipeline,
    pub(crate) _white_mask_texture: wgpu::Texture,
    pub(crate) white_mask_view: wgpu::TextureView,
}

pub(crate) struct EmbeddedGpuSurfaceRuntime {
    pub(crate) surface: GpuSurface,
    pub(crate) env: Environment,
    pub(crate) setup_complete: bool,
    pub(crate) output_format: wgpu::TextureFormat,
    pub(crate) output_size: (u32, u32),
    pub(crate) output_texture: Option<wgpu::Texture>,
    pub(crate) output_view: Option<wgpu::TextureView>,
    pub(crate) redraw_handle: RedrawHandle,
    pub(crate) start_time: Instant,
    pub(crate) last_frame_time: Instant,
}

#[derive(Clone)]
pub(crate) enum LayerShape {
    Rect(vello::kurbo::Rect),
    Path(vello::kurbo::BezPath),
}

#[derive(Clone)]
pub(crate) struct ActiveSceneLayer {
    pub(crate) alpha: f32,
    pub(crate) transform: vello::kurbo::Affine,
    pub(crate) shape: LayerShape,
}

#[derive(Clone)]
pub(crate) struct GpuSurfaceLayer {
    pub(crate) slot_index: usize,
    pub(crate) transform: vello::kurbo::Affine,
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) active_layers: Vec<ActiveSceneLayer>,
    pub(crate) direct_to_target: bool,
}

pub(crate) enum RenderLayer {
    Vello(vello::Scene),
    GpuSurface(GpuSurfaceLayer),
}

pub(crate) struct PreparedGpuSurfaceLayer {
    pub(crate) view: wgpu::TextureView,
    pub(crate) uniform_bytes: [u8; 80],
    pub(crate) needs_redraw: bool,
}

pub struct HydrolysisRenderTarget<'a> {
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
}

pub(crate) struct EmbeddedLayerTarget {
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) transform: vello::kurbo::Affine,
    pub(crate) bounds: vello::kurbo::Rect,
}

struct TextureLayerComposite<'a> {
    layer_view: &'a wgpu::TextureView,
    mask_view: &'a wgpu::TextureView,
    uniform_bytes: &'a [u8; 80],
    load_op: wgpu::LoadOp<wgpu::Color>,
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
            LayerShape::Path(path) => {
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
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_uniform"),
            size: 80,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
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
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            core::num::NonZeroU64::new(80)
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
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(GPU_SURFACE_COMPOSITOR_SHADER)),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
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
            multiview: None,
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
        let now = Instant::now();
        Self {
            surface,
            env: env.clone(),
            setup_complete: false,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            output_size: (1, 1),
            output_texture: None,
            output_view: None,
            redraw_handle: RedrawHandle::new(),
            start_time: now,
            last_frame_time: now - Duration::from_secs_f32(1.0 / 60.0),
        }
    }

    pub(crate) fn replace_surface(&mut self, surface: GpuSurface, env: &Environment) {
        let now = Instant::now();
        self.surface = surface;
        self.env = env.clone();
        self.setup_complete = false;
        self.output_format = wgpu::TextureFormat::Rgba8Unorm;
        self.output_size = (1, 1);
        self.output_texture = None;
        self.output_view = None;
        self.redraw_handle = RedrawHandle::new();
        self.start_time = now;
        self.last_frame_time = now - Duration::from_secs_f32(1.0 / 60.0);
    }

    pub(crate) fn take_external_redraw_request(&self) -> bool {
        self.redraw_handle.take_dirty()
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
        let output_format =
            select_embedded_surface_format(target.format, self.surface.get_surface_prefers_hdr());
        self.ensure_setup(device, queue, output_format);
        self.ensure_output_target(device, layer_width, layer_height, output_format);

        let texture = self
            .output_texture
            .as_ref()
            .expect("hydrolysis embedded GpuSurface missing output texture");
        let view = self
            .output_view
            .as_ref()
            .expect("hydrolysis embedded GpuSurface missing output view")
            .clone();
        let now = Instant::now();
        let elapsed = now.duration_since(self.start_time);
        let delta = now
            .duration_since(self.last_frame_time)
            .min(Duration::from_millis(100));
        self.last_frame_time = now;
        let mut frame = GpuFrame::new(
            device,
            queue,
            texture,
            view.clone(),
            output_format,
            layer_width,
            layer_height,
            PointerState::default(),
            GestureState::new(),
            elapsed,
            delta,
        );
        self.surface.render(&mut frame);
        let needs_redraw = frame.was_redraw_requested() || self.redraw_handle.take_dirty();
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
        self.ensure_setup(target.device, target.queue, target.format);

        let now = Instant::now();
        let elapsed = now.duration_since(self.start_time);
        let delta = now
            .duration_since(self.last_frame_time)
            .min(Duration::from_millis(100));
        self.last_frame_time = now;
        let mut frame = GpuFrame::new(
            target.device,
            target.queue,
            target.texture,
            target.view,
            target.format,
            target.width,
            target.height,
            PointerState::default(),
            GestureState::new(),
            elapsed,
            delta,
        );
        self.surface.render(&mut frame);
        frame.was_redraw_requested() || self.redraw_handle.take_dirty()
    }

    fn ensure_setup(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
    ) {
        if self.setup_complete && self.output_format == surface_format {
            return;
        }

        let ctx = GpuContext {
            adapter: None,
            device,
            queue,
            surface_format,
            msaa_samples: self.surface.get_msaa_max_samples().get(),
            pipeline_cache: None,
            redraw_handle: self.redraw_handle.clone(),
        };
        pollster::block_on(self.surface.setup(&ctx, &mut self.env));
        self.setup_complete = true;
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
    let prefers_hdr =
        waterui_graphics::gpu_surface::resolve_surface_hdr_preference(prefers_hdr_override);
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

    fn ensure_vello_layer_surface(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let size = (width, height);
        let needs_recreate = self
            .compositor
            .vello_layer_surface
            .as_ref()
            .is_none_or(|state| state.size != size);
        if !needs_recreate {
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_vello_layer_surface"),
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
        self.compositor.vello_layer_surface = Some(VelloLayerSurfaceState {
            size,
            _texture: texture,
            view,
        });
    }

    fn render_vello_layer_to_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &vello::Scene,
        width: u32,
        height: u32,
    ) -> wgpu::TextureView {
        self.ensure_vello_layer_surface(device, width, height);
        let view = self
            .compositor
            .vello_layer_surface
            .as_ref()
            .expect("hydrolysis renderer: missing vello layer surface state")
            .view
            .clone();
        let params = vello::RenderParams {
            base_color: vello::peniko::Color::TRANSPARENT,
            width,
            height,
            antialiasing_method: vello::AaConfig::Area,
        };
        self.vello_renderer
            .render_to_texture(device, queue, scene, &view, &params)
            .expect("hydrolysis renderer: failed to render vello layer scene");
        view
    }

    fn render_active_layers_mask_to_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        active_layers: &[ActiveSceneLayer],
    ) -> wgpu::TextureView {
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
        });
        drop(_pass);
        queue.submit(std::iter::once(encoder.finish()));
    }

    fn composite_texture_layer(
        &mut self,
        target: &HydrolysisRenderTarget<'_>,
        layer: TextureLayerComposite<'_>,
    ) {
        self.ensure_gpu_surface_compositor_state(target.device, target.queue, target.format);
        let compositor = self
            .compositor
            .gpu_surface_compositor
            .as_ref()
            .expect("hydrolysis renderer: missing gpu surface compositor state");

        target
            .queue
            .write_buffer(&compositor.uniform_buffer, 0, layer.uniform_bytes);
        let bind_group = target.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_bind_group"),
            layout: &compositor.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: compositor.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&compositor.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(layer.layer_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(layer.mask_view),
                },
            ],
        });

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
                    load: layer.load_op,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&compositor.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..6, 0..1);
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
        let transient_layer_count = if let Some(scene) = self
            .transient_scene
            .take()
            .filter(|scene| !scene.encoding().is_empty())
        {
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
            let needs_redraw = self
                .compositor
                .gpu_surface_slots
                .get_mut(layer.slot_index)
                .unwrap_or_else(|| {
                    panic!("hydrolysis gpu surface slot {} missing", layer.slot_index)
                })
                .render_direct_to_target(DirectGpuSurfaceTarget {
                    device: target.device,
                    queue: target.queue,
                    texture,
                    view: target.view.clone(),
                    format: target.format,
                    width: target.width,
                    height: target.height,
                });
            self.compositor.render_layers = render_layers;
            if needs_redraw {
                self.request_redraw();
            }
            return;
        }
        let mut needs_redraw = false;
        let mut is_first_layer = true;

        for layer in &render_layers {
            let load_op = if is_first_layer {
                wgpu::LoadOp::Clear(color_to_wgpu(target.base_color))
            } else {
                wgpu::LoadOp::Load
            };
            is_first_layer = false;
            match layer {
                RenderLayer::Vello(scene) => {
                    let view = self.render_vello_layer_to_texture(
                        target.device,
                        target.queue,
                        scene,
                        target.width,
                        target.height,
                    );
                    let mask_view = self.default_compositor_mask_view(
                        target.device,
                        target.queue,
                        target.format,
                    );
                    self.composite_texture_layer(
                        &target,
                        TextureLayerComposite {
                            layer_view: &view,
                            mask_view: &mask_view,
                            uniform_bytes: &fullscreen_uniform,
                            load_op,
                        },
                    );
                }
                RenderLayer::GpuSurface(layer) => {
                    let prepared = self
                        .compositor
                        .gpu_surface_slots
                        .get_mut(layer.slot_index)
                        .unwrap_or_else(|| {
                            panic!("hydrolysis gpu surface slot {} missing", layer.slot_index)
                        })
                        .prepare_layer(
                            target.device,
                            target.queue,
                            EmbeddedLayerTarget {
                                format: target.format,
                                width: target.width,
                                height: target.height,
                                transform: layer.transform,
                                bounds: layer.bounds,
                            },
                        );
                    if prepared.needs_redraw {
                        needs_redraw = true;
                    }
                    let mask_view = if layer.active_layers.is_empty() {
                        self.default_compositor_mask_view(
                            target.device,
                            target.queue,
                            target.format,
                        )
                    } else {
                        self.render_active_layers_mask_to_texture(
                            target.device,
                            target.queue,
                            target.width,
                            target.height,
                            &layer.active_layers,
                        )
                    };
                    self.composite_texture_layer(
                        &target,
                        TextureLayerComposite {
                            layer_view: &prepared.view,
                            mask_view: &mask_view,
                            uniform_bytes: &prepared.uniform_bytes,
                            load_op,
                        },
                    );
                }
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
