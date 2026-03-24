use super::*;

const GPU_SURFACE_COMPOSITOR_SHADER: &str = include_str!("../shaders/gpu_surface_compositor.wgsl");

#[derive(Default)]
pub(super) struct Compositor {
    pub(super) vello_layer_surface: Option<VelloLayerSurfaceState>,
    pub(super) gpu_surface_compositor: Option<GpuSurfaceCompositorState>,
    pub(super) gpu_surface_slots: Vec<EmbeddedGpuSurfaceRuntime>,
    pub(super) gpu_surface_cursor: usize,
    pub(super) render_layers: Vec<RenderLayer>,
    pub(super) active_scene_layers: Vec<ActiveSceneLayer>,
    pub(super) active_filter_images: Vec<vello::peniko::ImageData>,
}

pub(super) struct VelloLayerSurfaceState {
    pub(super) size: (u32, u32),
    pub(super) _texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
}

pub(super) struct GpuSurfaceCompositorState {
    pub(super) target_format: wgpu::TextureFormat,
    pub(super) uniform_buffer: wgpu::Buffer,
    pub(super) sampler: wgpu::Sampler,
    pub(super) bind_group_layout: wgpu::BindGroupLayout,
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) _white_mask_texture: wgpu::Texture,
    pub(super) white_mask_view: wgpu::TextureView,
}

pub(super) struct EmbeddedGpuSurfaceRuntime {
    pub(super) surface: GpuSurface,
    pub(super) env: Environment,
    pub(super) setup_complete: bool,
    pub(super) output_format: wgpu::TextureFormat,
    pub(super) output_size: (u32, u32),
    pub(super) output_texture: Option<wgpu::Texture>,
    pub(super) output_view: Option<wgpu::TextureView>,
    pub(super) redraw_handle: RedrawHandle,
    pub(super) start_time: Instant,
    pub(super) last_frame_time: Instant,
}

#[derive(Clone)]
pub(super) enum LayerShape {
    Rect(vello::kurbo::Rect),
    Path(vello::kurbo::BezPath),
}

#[derive(Clone)]
pub(super) struct ActiveSceneLayer {
    pub(super) alpha: f32,
    pub(super) transform: vello::kurbo::Affine,
    pub(super) shape: LayerShape,
}

#[derive(Clone)]
pub(super) struct GpuSurfaceLayer {
    pub(super) slot_index: usize,
    pub(super) transform: vello::kurbo::Affine,
    pub(super) bounds: vello::kurbo::Rect,
    pub(super) active_layers: Vec<ActiveSceneLayer>,
}

pub(super) enum RenderLayer {
    Vello(vello::Scene),
    GpuSurface(GpuSurfaceLayer),
}

pub(super) struct PreparedGpuSurfaceLayer {
    pub(super) view: wgpu::TextureView,
    pub(super) uniform_bytes: [u8; 64],
    pub(super) needs_redraw: bool,
}

impl ActiveSceneLayer {
    pub(super) fn push_to_scene(&self, scene: &mut vello::Scene) {
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
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_uniform"),
            size: 64,
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
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            core::num::NonZeroU64::new(64)
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

    pub(super) fn ensure_target_format(
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
    pub(super) fn new(surface: GpuSurface, env: &Environment) -> Self {
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

    pub(super) fn replace_surface(&mut self, surface: GpuSurface, env: &Environment) {
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

    pub(super) fn take_external_redraw_request(&self) -> bool {
        self.redraw_handle.take_dirty()
    }

    pub(super) fn prepare_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        target_width: u32,
        target_height: u32,
        transform: vello::kurbo::Affine,
        bounds: vello::kurbo::Rect,
    ) -> PreparedGpuSurfaceLayer {
        let top_left = transform * vello::kurbo::Point::new(bounds.x0, bounds.y0);
        let top_right = transform * vello::kurbo::Point::new(bounds.x1, bounds.y0);
        let bottom_right = transform * vello::kurbo::Point::new(bounds.x1, bounds.y1);
        let bottom_left = transform * vello::kurbo::Point::new(bounds.x0, bounds.y1);

        let layer_width =
            edge_length_in_pixels(top_left, top_right, target_width, target_height).max(1);
        let layer_height =
            edge_length_in_pixels(top_left, bottom_left, target_width, target_height).max(1);
        let output_format =
            select_embedded_surface_format(target_format, self.surface.get_surface_prefers_hdr());
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
            point_to_clip(top_left, target_width, target_height),
            point_to_clip(top_right, target_width, target_height),
            point_to_clip(bottom_right, target_width, target_height),
            point_to_clip(bottom_left, target_width, target_height),
        ];

        PreparedGpuSurfaceLayer {
            view,
            uniform_bytes: encode_compositor_uniform(corners),
            needs_redraw,
        }
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

fn encode_compositor_uniform(corners: [[f32; 2]; 4]) -> [u8; 64] {
    let uvs = [[0.0f32, 0.0f32], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let mut bytes = [0u8; 64];
    for (index, corner) in corners.iter().enumerate() {
        let base = index * 16;
        write_f32(&mut bytes, base, corner[0]);
        write_f32(&mut bytes, base + 4, corner[1]);
        write_f32(&mut bytes, base + 8, uvs[index][0]);
        write_f32(&mut bytes, base + 12, uvs[index][1]);
    }
    bytes
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

impl HydrolysisRenderer {
    pub fn render_scene_to_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        target_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        base_color: vello::peniko::Color,
    ) {
        self.render_scene_to_surface(
            device,
            queue,
            target,
            target_format,
            width,
            height,
            base_color,
        );
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
        self.compositor.gpu_surface_compositor
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
        self.compositor.gpu_surface_compositor
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
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        target_format: wgpu::TextureFormat,
        layer_view: &wgpu::TextureView,
        mask_view: &wgpu::TextureView,
        uniform_bytes: &[u8; 64],
        load_op: wgpu::LoadOp<wgpu::Color>,
    ) {
        self.ensure_gpu_surface_compositor_state(device, queue, target_format);
        let compositor = self
            .compositor
            .gpu_surface_compositor
            .as_ref()
            .expect("hydrolysis renderer: missing gpu surface compositor state");

        queue.write_buffer(&compositor.uniform_buffer, 0, uniform_bytes);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                    resource: wgpu::BindingResource::TextureView(layer_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(mask_view),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_encoder"),
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: load_op,
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
        queue.submit(std::iter::once(encoder.finish()));
    }

    pub fn render_scene_to_surface(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        target_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        base_color: vello::peniko::Color,
    ) {
        assert!(
            matches!(
                target_format.remove_srgb_suffix(),
                wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
            ) || matches!(
                target_format,
                wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
            ),
            "hydrolysis renderer: unsupported surface format {target_format:?}"
        );

        self.flush_vello_scene_layer();
        let fullscreen_uniform =
            encode_compositor_uniform([[-1.0, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]]);
        let render_layers = core::mem::take(&mut self.compositor.render_layers);
        if render_layers.is_empty() {
            self.clear_target_surface(device, queue, target, base_color);
            return;
        }
        let mut needs_redraw = false;
        let mut is_first_layer = true;

        for layer in &render_layers {
            let load_op = if is_first_layer {
                wgpu::LoadOp::Clear(color_to_wgpu(base_color))
            } else {
                wgpu::LoadOp::Load
            };
            is_first_layer = false;
            match layer {
                RenderLayer::Vello(scene) => {
                    let view =
                        self.render_vello_layer_to_texture(device, queue, scene, width, height);
                    let mask_view = self.default_compositor_mask_view(device, queue, target_format);
                    self.composite_texture_layer(
                        device,
                        queue,
                        target,
                        target_format,
                        &view,
                        &mask_view,
                        &fullscreen_uniform,
                        load_op,
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
                            device,
                            queue,
                            target_format,
                            width,
                            height,
                            layer.transform,
                            layer.bounds,
                        );
                    if prepared.needs_redraw {
                        needs_redraw = true;
                    }
                    let mask_view = if layer.active_layers.is_empty() {
                        self.default_compositor_mask_view(device, queue, target_format)
                    } else {
                        self.render_active_layers_mask_to_texture(
                            device,
                            queue,
                            width,
                            height,
                            &layer.active_layers,
                        )
                    };
                    self.composite_texture_layer(
                        device,
                        queue,
                        target,
                        target_format,
                        &prepared.view,
                        &mask_view,
                        &prepared.uniform_bytes,
                        load_op,
                    );
                }
            }
        }
        self.compositor.render_layers = render_layers;

        if needs_redraw {
            self.request_redraw();
        }
    }
}
