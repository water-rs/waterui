//! Cinematic HDR Flame - multi-pass renderer on top of GpuSurface.
//!
//! Passes:
//! 1) Flame HDR render -> hdr_tex (Rgba16Float)
//! 2) Bright extract + downsample -> bloom_tex (Rgba16Float, half res)
//! 3) Gaussian blur H -> ping
//! 4) Gaussian blur V -> bloom_tex
//! 5) Composite (hdr + bloom) + ACES tonemap -> swapchain surface

use waterui::{
    AnyView, Environment, View,
    graphics::{GpuContext, GpuFrame, GpuRenderer, GpuSurface, bytemuck, wgpu},
};

pub struct FlameSurface {
    inner: GpuSurface,
}

impl core::fmt::Debug for FlameSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FlameSurface").finish_non_exhaustive()
    }
}

impl FlameSurface {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: GpuSurface::new(FlameRenderer::new()),
        }
    }
}

impl View for FlameSurface {
    fn body(self, _env: &Environment) -> impl View {
        self.inner
    }
}

pub fn init() -> Environment {
    Environment::new()
}

pub fn main() -> impl View {
    FlameSurface::new()
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SceneUniforms {
    time: f32,       // 0
    _pad0: f32,      // 4  (align vec2)
    res_x: f32,      // 8
    res_y: f32,      // 12
    frame_seed: f32, // 16 (grain jitter etc.)
    _pad1: f32,      // 20
}
// 24 bytes total (matches ShaderSurface-style alignment)

unsafe impl bytemuck::Pod for SceneUniforms {}
unsafe impl bytemuck::Zeroable for SceneUniforms {}

#[repr(C)]
#[derive(Clone, Copy)]
struct BloomParams {
    // xy = source resolution, zw = inv source resolution
    src_res_x: f32,
    src_res_y: f32,
    inv_src_x: f32,
    inv_src_y: f32,

    // threshold/knee/intensity/pad
    threshold: f32,
    knee: f32,
    intensity: f32,
    _pad0: f32,

    // blur direction (dx, dy) in texel units + pad
    dir_x: f32,
    dir_y: f32,
    _pad1: f32,
    _pad2: f32,
}
// 48 bytes

unsafe impl bytemuck::Pod for BloomParams {}
unsafe impl bytemuck::Zeroable for BloomParams {}

struct FlameRenderer {
    // pipelines
    flame_pipeline: Option<wgpu::RenderPipeline>,
    extract_pipeline: Option<wgpu::RenderPipeline>,
    blur_pipeline: Option<wgpu::RenderPipeline>,
    composite_pipeline: Option<wgpu::RenderPipeline>,

    // bind group layouts
    scene_bgl: Option<wgpu::BindGroupLayout>,
    tex_bgl: Option<wgpu::BindGroupLayout>,
    bloom_bgl: Option<wgpu::BindGroupLayout>,

    // buffers
    scene_ubo: Option<wgpu::Buffer>,
    bloom_ubo: Option<wgpu::Buffer>,
    scene_bg: Option<wgpu::BindGroup>,
    bloom_bg: Option<wgpu::BindGroup>,

    // textures
    hdr_tex: Option<wgpu::Texture>,
    hdr_view: Option<wgpu::TextureView>,

    bloom_tex: Option<wgpu::Texture>,
    bloom_view: Option<wgpu::TextureView>,

    ping_tex: Option<wgpu::Texture>,
    ping_view: Option<wgpu::TextureView>,

    sampler: Option<wgpu::Sampler>,

    // texture bind groups
    hdr_sample_bg: Option<wgpu::BindGroup>,   // hdr as sampled
    bloom_sample_bg: Option<wgpu::BindGroup>, // bloom as sampled
    ping_sample_bg: Option<wgpu::BindGroup>,  // ping as sampled

    // size tracking
    size: (u32, u32),
    surface_format: Option<wgpu::TextureFormat>,

    start: std::time::Instant,
    frame_index: u32,
}

impl FlameRenderer {
    fn new() -> Self {
        Self {
            flame_pipeline: None,
            extract_pipeline: None,
            blur_pipeline: None,
            composite_pipeline: None,

            scene_bgl: None,
            tex_bgl: None,
            bloom_bgl: None,

            scene_ubo: None,
            bloom_ubo: None,
            scene_bg: None,
            bloom_bg: None,

            hdr_tex: None,
            hdr_view: None,

            bloom_tex: None,
            bloom_view: None,

            ping_tex: None,
            ping_view: None,

            sampler: None,

            hdr_sample_bg: None,
            bloom_sample_bg: None,
            ping_sample_bg: None,

            size: (0, 0),
            surface_format: None,

            start: std::time::Instant::now(),
            frame_index: 0,
        }
    }

    fn ensure_layouts(&mut self, device: &wgpu::Device) {
        if self.scene_bgl.is_none() {
            let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Flame Scene BGL"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: core::num::NonZeroU64::new(core::mem::size_of::<
                            SceneUniforms,
                        >()
                            as u64),
                    },
                    count: None,
                }],
            });

            let tex_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Flame Texture BGL"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

            let bloom_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Flame Bloom Params BGL"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: core::num::NonZeroU64::new(
                            core::mem::size_of::<BloomParams>() as u64,
                        ),
                    },
                    count: None,
                }],
            });

            self.scene_bgl = Some(scene_bgl);
            self.tex_bgl = Some(tex_bgl);
            self.bloom_bgl = Some(bloom_bgl);
        }
    }

    fn ensure_buffers(&mut self, device: &wgpu::Device) {
        if self.scene_ubo.is_none() {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Flame Scene UBO"),
                size: core::mem::size_of::<SceneUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.scene_ubo = Some(buf);
        }

        if self.bloom_ubo.is_none() {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Flame Bloom UBO"),
                size: core::mem::size_of::<BloomParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.bloom_ubo = Some(buf);
        }

        if self.sampler.is_none() {
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Flame Linear Sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                ..Default::default()
            });
            self.sampler = Some(sampler);
        }

        if self.scene_bg.is_none() {
            let bgl = self.scene_bgl.as_ref().unwrap();
            let ubo = self.scene_ubo.as_ref().unwrap();
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Flame Scene BG"),
                layout: bgl,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ubo.as_entire_binding(),
                }],
            });
            self.scene_bg = Some(bg);
        }

        if self.bloom_bg.is_none() {
            let bgl = self.bloom_bgl.as_ref().unwrap();
            let ubo = self.bloom_ubo.as_ref().unwrap();
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Flame Bloom BG"),
                layout: bgl,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ubo.as_entire_binding(),
                }],
            });
            self.bloom_bg = Some(bg);
        }
    }

    fn create_fullscreen_pipeline(
        device: &wgpu::Device,
        label: &str,
        shader_src: &str,
        entry_fs: &str,
        target_format: wgpu::TextureFormat,
        bgls: &[&wgpu::BindGroupLayout],
    ) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });

        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&format!("{label} Layout")),
            bind_group_layouts: bgls,
            push_constant_ranges: &[],
        });

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(entry_fs),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    fn ensure_textures(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.size == (width, height) && self.hdr_tex.is_some() {
            return;
        }
        self.size = (width, height);

        let hdr_format = wgpu::TextureFormat::Rgba16Float;
        let (bw, bh) = ((width.max(2) / 2).max(1), (height.max(2) / 2).max(1));

        let hdr_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Flame HDR Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: hdr_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let hdr_view = hdr_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let bloom_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Flame Bloom Texture"),
            size: wgpu::Extent3d {
                width: bw,
                height: bh,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: hdr_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let bloom_view = bloom_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let ping_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Flame Bloom Ping Texture"),
            size: wgpu::Extent3d {
                width: bw,
                height: bh,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: hdr_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let ping_view = ping_tex.create_view(&wgpu::TextureViewDescriptor::default());

        self.hdr_tex = Some(hdr_tex);
        self.hdr_view = Some(hdr_view);

        self.bloom_tex = Some(bloom_tex);
        self.bloom_view = Some(bloom_view);

        self.ping_tex = Some(ping_tex);
        self.ping_view = Some(ping_view);

        // rebuild sampled bind groups (depend on views)
        self.hdr_sample_bg = None;
        self.bloom_sample_bg = None;
        self.ping_sample_bg = None;
    }

    fn ensure_texture_bind_groups(&mut self, device: &wgpu::Device) {
        if self.hdr_sample_bg.is_some()
            && self.bloom_sample_bg.is_some()
            && self.ping_sample_bg.is_some()
        {
            return;
        }
        let tex_bgl = self.tex_bgl.as_ref().unwrap();
        let sampler = self.sampler.as_ref().unwrap();

        let hdr_view = self.hdr_view.as_ref().unwrap();
        let bloom_view = self.bloom_view.as_ref().unwrap();
        let ping_view = self.ping_view.as_ref().unwrap();

        self.hdr_sample_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flame HDR Sample BG"),
            layout: tex_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        }));

        self.bloom_sample_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flame Bloom Sample BG"),
            layout: tex_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(bloom_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        }));

        self.ping_sample_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Flame Ping Sample BG"),
            layout: tex_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(ping_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        }));
    }

    fn ensure_pipelines(&mut self, device: &wgpu::Device, surface_format: wgpu::TextureFormat) {
        if self.flame_pipeline.is_some()
            && self.extract_pipeline.is_some()
            && self.blur_pipeline.is_some()
            && self.composite_pipeline.is_some()
            && self.surface_format == Some(surface_format)
        {
            return;
        }

        self.surface_format = Some(surface_format);

        let scene_bgl = self.scene_bgl.as_ref().unwrap();
        let tex_bgl = self.tex_bgl.as_ref().unwrap();
        let bloom_bgl = self.bloom_bgl.as_ref().unwrap();

        // Pass 1: flame -> HDR
        self.flame_pipeline = Some(Self::create_fullscreen_pipeline(
            device,
            "Flame Pass (HDR)",
            FLAME_PASS_WGSL,
            "fs_flame",
            wgpu::TextureFormat::Rgba16Float,
            &[scene_bgl],
        ));

        // Pass 2: extract+downsample: HDR -> bloom half
        self.extract_pipeline = Some(Self::create_fullscreen_pipeline(
            device,
            "Bloom Extract Pass",
            EXTRACT_BLUR_COMPOSITE_WGSL,
            "fs_extract",
            wgpu::TextureFormat::Rgba16Float,
            &[tex_bgl, bloom_bgl],
        ));

        // Pass 3/4: blur (same shader, direction in BloomParams)
        self.blur_pipeline = Some(Self::create_fullscreen_pipeline(
            device,
            "Bloom Blur Pass",
            EXTRACT_BLUR_COMPOSITE_WGSL,
            "fs_blur",
            wgpu::TextureFormat::Rgba16Float,
            &[tex_bgl, bloom_bgl],
        ));

        // Pass 5: composite -> surface
        self.composite_pipeline = Some(Self::create_fullscreen_pipeline(
            device,
            "Composite + Tonemap Pass",
            EXTRACT_BLUR_COMPOSITE_WGSL,
            "fs_composite",
            surface_format,
            &[tex_bgl, tex_bgl, bloom_bgl, scene_bgl],
        ));
    }
}

impl GpuRenderer for FlameRenderer {
    fn setup(&mut self, ctx: &GpuContext) {
        self.ensure_layouts(ctx.device);
        self.ensure_buffers(ctx.device);
        self.ensure_pipelines(ctx.device, ctx.surface_format);
        self.start = std::time::Instant::now();
        self.frame_index = 0;
    }

    fn resize(&mut self, _width: u32, _height: u32) {
        // textures recreated lazily in render
        self.size = (0, 0);
    }

    fn render(&mut self, frame: &GpuFrame) {
        self.ensure_layouts(frame.device);
        self.ensure_buffers(frame.device);

        self.ensure_textures(frame.device, frame.width, frame.height);
        self.ensure_texture_bind_groups(frame.device);

        // if swapchain format changes, rebuild composite pipeline
        self.ensure_pipelines(frame.device, frame.format);

        let elapsed = self.start.elapsed().as_secs_f32();
        self.frame_index = self.frame_index.wrapping_add(1);

        // update scene uniforms
        let scene = SceneUniforms {
            time: elapsed,
            _pad0: 0.0,
            res_x: frame.width as f32,
            res_y: frame.height as f32,
            frame_seed: (self.frame_index as f32) * 0.1234,
            _pad1: 0.0,
        };
        frame.queue.write_buffer(
            self.scene_ubo.as_ref().unwrap(),
            0,
            bytemuck::bytes_of(&scene),
        );

        let (bw, bh) = (
            (frame.width.max(2) / 2).max(1),
            (frame.height.max(2) / 2).max(1),
        );
        let inv_bw = 1.0 / (bw as f32);
        let inv_bh = 1.0 / (bh as f32);

        // common bloom params (threshold/knee/intensity)
        // threshold: brighter sparks/white core generate bloom; knee softens cutoff.
        let mut bloom = BloomParams {
            src_res_x: bw as f32,
            src_res_y: bh as f32,
            inv_src_x: inv_bw,
            inv_src_y: inv_bh,
            threshold: 1.25,
            knee: 0.65,
            intensity: 0.85,
            _pad0: 0.0,
            dir_x: 1.0,
            dir_y: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
        };

        let encoder_label = "Flame HDR Encoder";
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some(encoder_label),
            });

        // ---------- Pass 1: render flame into HDR texture ----------
        {
            let hdr_view = self.hdr_view.as_ref().unwrap();
            let pipeline = self.flame_pipeline.as_ref().unwrap();
            let scene_bg = self.scene_bg.as_ref().unwrap();

            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Pass1 Flame HDR"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: hdr_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, scene_bg, &[]);
            rp.draw(0..6, 0..1);
        }

        // ---------- Pass 2: bright extract + downsample -> bloom_tex ----------
        {
            let bloom_view = self.bloom_view.as_ref().unwrap();
            let pipeline = self.extract_pipeline.as_ref().unwrap();
            let hdr_bg = self.hdr_sample_bg.as_ref().unwrap();
            let bloom_bg = self.bloom_bg.as_ref().unwrap();

            // For extract pass, src texture is HDR full-res; we still use bloom params for threshold.
            // We set src_res to FULL-res for correct sampling in shader.
            bloom.src_res_x = frame.width as f32;
            bloom.src_res_y = frame.height as f32;
            bloom.inv_src_x = 1.0 / (frame.width as f32);
            bloom.inv_src_y = 1.0 / (frame.height as f32);
            bloom.dir_x = 0.0;
            bloom.dir_y = 0.0;

            frame.queue.write_buffer(
                self.bloom_ubo.as_ref().unwrap(),
                0,
                bytemuck::bytes_of(&bloom),
            );

            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Pass2 Extract+Downsample"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: bloom_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, hdr_bg, &[]);
            rp.set_bind_group(1, bloom_bg, &[]);
            rp.draw(0..6, 0..1);
        }

        // ---------- Pass 3: blur horizontal bloom_tex -> ping ----------
        {
            let ping_view = self.ping_view.as_ref().unwrap();
            let pipeline = self.blur_pipeline.as_ref().unwrap();
            let bloom_sample = self.bloom_sample_bg.as_ref().unwrap();
            let bloom_bg = self.bloom_bg.as_ref().unwrap();

            bloom.src_res_x = bw as f32;
            bloom.src_res_y = bh as f32;
            bloom.inv_src_x = inv_bw;
            bloom.inv_src_y = inv_bh;
            bloom.dir_x = 1.0;
            bloom.dir_y = 0.0;
            frame.queue.write_buffer(
                self.bloom_ubo.as_ref().unwrap(),
                0,
                bytemuck::bytes_of(&bloom),
            );

            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Pass3 Blur H"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: ping_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, bloom_sample, &[]);
            rp.set_bind_group(1, bloom_bg, &[]);
            rp.draw(0..6, 0..1);
        }

        // ---------- Pass 4: blur vertical ping -> bloom_tex ----------
        {
            let bloom_view = self.bloom_view.as_ref().unwrap();
            let pipeline = self.blur_pipeline.as_ref().unwrap();
            let ping_sample = self.ping_sample_bg.as_ref().unwrap();
            let bloom_bg = self.bloom_bg.as_ref().unwrap();

            bloom.dir_x = 0.0;
            bloom.dir_y = 1.0;
            frame.queue.write_buffer(
                self.bloom_ubo.as_ref().unwrap(),
                0,
                bytemuck::bytes_of(&bloom),
            );

            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Pass4 Blur V"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: bloom_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, ping_sample, &[]);
            rp.set_bind_group(1, bloom_bg, &[]);
            rp.draw(0..6, 0..1);
        }

        // ---------- Pass 5: composite hdr + bloom -> surface ----------
        {
            let pipeline = self.composite_pipeline.as_ref().unwrap();
            let hdr_bg = self.hdr_sample_bg.as_ref().unwrap();
            let bloom_bg_sample = self.bloom_sample_bg.as_ref().unwrap();
            let bloom_params_bg = self.bloom_bg.as_ref().unwrap();
            let scene_bg = self.scene_bg.as_ref().unwrap();

            // restore src_res for bloom texture in composite
            bloom.src_res_x = bw as f32;
            bloom.src_res_y = bh as f32;
            bloom.inv_src_x = inv_bw;
            bloom.inv_src_y = inv_bh;
            bloom.dir_x = 0.0;
            bloom.dir_y = 0.0;
            frame.queue.write_buffer(
                self.bloom_ubo.as_ref().unwrap(),
                0,
                bytemuck::bytes_of(&bloom),
            );

            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Pass5 Composite+Tonemap"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(pipeline);
            // group0 = hdr, group1 = bloom, group2 = bloom params, group3 = scene
            rp.set_bind_group(0, hdr_bg, &[]);
            rp.set_bind_group(1, bloom_bg_sample, &[]);
            rp.set_bind_group(2, bloom_params_bg, &[]);
            rp.set_bind_group(3, scene_bg, &[]);
            rp.draw(0..6, 0..1);
        }

        frame.queue.submit(std::iter::once(encoder.finish()));
    }
}

waterui_ffi::export!();

// -------------------- WGSL --------------------

const FULLSCREEN_VS: &str = r#"
struct VSOut {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VSOut {
  var pos = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>( 1.0, -1.0),
    vec2<f32>(-1.0,  1.0),
    vec2<f32>(-1.0,  1.0),
    vec2<f32>( 1.0, -1.0),
    vec2<f32>( 1.0,  1.0)
  );

  var o: VSOut;
  o.position = vec4<f32>(pos[i], 0.0, 1.0);
  o.uv = (pos[i] + 1.0) * 0.5; // bottom-left (0,0)
  return o;
}
"#;

const FLAME_PASS_WGSL: &str = r#"
    struct VSOut {
      @builtin(position) position: vec4<f32>,
      @location(0) uv: vec2<f32>,
    };

    @vertex
    fn vs_main(@builtin(vertex_index) i: u32) -> VSOut {
      var pos = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0)
      );

      var o: VSOut;
      o.position = vec4<f32>(pos[i], 0.0, 1.0);
      o.uv = (pos[i] + 1.0) * 0.5; // bottom-left (0,0)
      return o;
    }
struct SceneUniforms {
  time: f32,
  _pad0: f32,
  res_x: f32,
  res_y: f32,
  frame_seed: f32,
  _pad1: f32,
};

@group(0) @binding(0) var<uniform> u: SceneUniforms;

fn hash12(p: vec2<f32>) -> f32 {
  return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn noise2(p: vec2<f32>) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u2 = f * f * (3.0 - 2.0 * f);
  let a = hash12(i);
  let b = hash12(i + vec2<f32>(1.0, 0.0));
  let c = hash12(i + vec2<f32>(0.0, 1.0));
  let d = hash12(i + vec2<f32>(1.0, 1.0));
  return mix(mix(a, b, u2.x), mix(c, d, u2.x), u2.y);
}

fn fbm(p: vec2<f32>) -> f32 {
  var v = 0.0;
  var a = 0.5;
  var x = p;
  for (var i = 0; i < 6; i = i + 1) {
    v += a * noise2(x);
    x = mat2x2<f32>(1.7, 1.2, -1.2, 1.7) * x + 0.11;
    a *= 0.5;
  }
  return v;
}

fn flow(p: vec2<f32>) -> vec2<f32> {
  let e = 0.003;
  let n1 = fbm(p + vec2<f32>( e, 0.0));
  let n2 = fbm(p + vec2<f32>(-e, 0.0));
  let n3 = fbm(p + vec2<f32>(0.0,  e));
  let n4 = fbm(p + vec2<f32>(0.0, -e));
  let gx = (n1 - n2) / (2.0 * e);
  let gy = (n3 - n4) / (2.0 * e);
  return normalize(vec2<f32>(gy, -gx) + 1e-5);
}

// blackbody-ish HDR ramp
fn fire_color(t: f32) -> vec3<f32> {
  let x = clamp(t, 0.0, 1.0);
  let c0 = vec3<f32>(0.02, 0.02, 0.03);
  let c1 = vec3<f32>(0.75, 0.08, 0.01);
  let c2 = vec3<f32>(1.60, 0.35, 0.02);
  let c3 = vec3<f32>(3.50, 1.30, 0.12);
  let c4 = vec3<f32>(12.0, 9.0, 4.5); // HDR white core
  let a = smoothstep(0.00, 0.30, x);
  let b = smoothstep(0.18, 0.60, x);
  let c = smoothstep(0.45, 0.90, x);
  let d = smoothstep(0.70, 1.00, x);
  var col = mix(c0, c1, a);
  col = mix(col, c2, b);
  col = mix(col, c3, c);
  col = mix(col, c4, d);
  return col;
}

@fragment
fn fs_flame(i: VSOut) -> @location(0) vec4<f32> {
  let t = u.time;

  let res = vec2<f32>(max(u.res_x, 1.0), max(u.res_y, 1.0));
  let aspect = res.x / res.y;

  // camera space
  var p = vec2<f32>((i.uv.x - 0.5) * aspect, i.uv.y - 0.02);

  // anchor flame lower third
  p.y = p.y * 1.25;

  // heat haze refraction
  let haze_n = fbm(p * 2.2 + vec2<f32>(0.0, -t * 0.9));
  p.x += (haze_n - 0.5) * (0.012 + 0.030 * smoothstep(0.10, 1.0, p.y));

  // curl-ish advection
  let f = flow(p * 1.35 + vec2<f32>(0.0, -t * 0.30));
  p += 0.12 * f * (0.35 + 0.65 * fbm(p * 1.9 + t * 0.12));
  p.x += 0.03 * sin(t * 1.2 + p.y * 7.0);

  // flame cone
  let y = p.y;
  let width = mix(0.58, 0.06, clamp(y, 0.0, 1.0));
  let cone = smoothstep(width, width - 0.09, abs(p.x));

  // multi-scale tongues
  let n_lo = fbm(vec2<f32>(p.x * 3.0,  p.y * 2.3) + vec2<f32>(0.0, -t * 1.35));
  let n_hi = fbm(vec2<f32>(p.x * 10.5, p.y * 5.4) + vec2<f32>(13.0, -t * 3.10));
  let tear = fbm(vec2<f32>(p.x * 5.0,  p.y * 1.2) + vec2<f32>(t * 0.4, -t * 0.7));

  let tongues = smoothstep(0.18, 1.0, n_lo * 0.85 + n_hi * 0.55);
  let breakup = smoothstep(0.15, 0.95, tongues + (tear - 0.5) * 0.35);

  let h = smoothstep(1.18, 0.05, y);
  var body = cone * breakup * h;
  body = pow(clamp(body, 0.0, 1.0), 1.6);

  // hot core
  let core_w = width * 0.33;
  var core = 1.0 - smoothstep(0.0, 1.0, abs(p.x) / (core_w + 1e-4));
  core *= smoothstep(0.95, 0.12, y);
  core *= smoothstep(0.20, 0.95, body);
  core = pow(clamp(core, 0.0, 1.0), 2.9);

  // smoke (darker, broader, slower)
  let smoke_n = fbm(vec2<f32>(p.x * 1.2, p.y * 1.0) + vec2<f32>(0.0, -t * 0.22));
  let smoke = smoothstep(0.55, 0.95, smoke_n) * smoothstep(0.05, 1.05, y) * 0.20;

  // embers
  var sparks = vec3<f32>(0.0);
  for (var k = 0; k < 7; k = k + 1) {
    let fk = f32(k);
    let q = vec2<f32>(p.x * 24.0 + fk * 7.3, p.y * 34.0 - t * (3.0 + fk * 0.35));
    let cell = floor(q);
    let rnd = hash12(cell + fk * 19.7);
    let local = fract(q) - 0.5;
    let d = length(local);
    let s = smoothstep(0.11, 0.0, d) * smoothstep(0.05, 0.85, body);
    let flick = 0.6 + 0.4 * sin(t * 11.0 + rnd * 6.283);
    sparks += vec3<f32>(1.3, 0.9, 0.35) * s * flick * 0.35;
  }

  // emissive HDR
  let heat = clamp(body * 1.10 + core * 1.10, 0.0, 1.0);
  var col = fire_color(heat) * (0.10 + 2.6 * body + 3.2 * core);

  // blue base (fuel-rich look)
  let blue = smoothstep(0.65, 0.05, y) * smoothstep(0.20, 0.85, core);
  col += vec3<f32>(0.10, 0.20, 0.60) * blue * 0.75;

  // smoke darkening
  col = mix(col, col * vec3<f32>(0.55, 0.58, 0.62), smoke);

  col += sparks;

  // subtle environment glow (still HDR)
  let glow = exp(-length(vec2<f32>(p.x, y - 0.25)) * 2.5) * 0.25;
  col += vec3<f32>(1.2, 0.45, 0.12) * glow;

  return vec4<f32>(col, 1.0);
}
"#;

const EXTRACT_BLUR_COMPOSITE_WGSL: &str = r#"
    struct VSOut {
      @builtin(position) position: vec4<f32>,
      @location(0) uv: vec2<f32>,
    };

    @vertex
    fn vs_main(@builtin(vertex_index) i: u32) -> VSOut {
      var pos = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0)
      );

      var o: VSOut;
      o.position = vec4<f32>(pos[i], 0.0, 1.0);
      o.uv = (pos[i] + 1.0) * 0.5; // bottom-left (0,0)
      return o;
    }
struct BloomParams {
  src_res_x: f32,
  src_res_y: f32,
  inv_src_x: f32,
  inv_src_y: f32,
  threshold: f32,
  knee: f32,
  intensity: f32,
  _pad0: f32,
  dir_x: f32,
  dir_y: f32,
  _pad1: f32,
  _pad2: f32,
};

struct SceneUniforms {
  time: f32,
  _pad0: f32,
  res_x: f32,
  res_y: f32,
  frame_seed: f32,
  _pad1: f32,
};

@group(0) @binding(0) var t_src: texture_2d<f32>;
@group(0) @binding(1) var s_src: sampler;

@group(1) @binding(0) var<uniform> p: BloomParams;

// Composite uses two textures; we’ll bind them as:
// group0: hdr, group1: bloom, group2: params, group3: scene
@group(3) @binding(0) var<uniform> scene: SceneUniforms;

// soft knee threshold
fn soft_threshold(c: vec3<f32>, threshold: f32, knee: f32) -> vec3<f32> {
  let l = max(max(c.r, c.g), c.b);
  let t = threshold;
  let k = knee;
  // smooth transition around threshold
  let x = max(l - t, 0.0);
  let y = x * x / (k * k + 1e-5);
  let w = x / (x + k + 1e-5);
  let m = mix(y, x, w);
  return c * (m / (l + 1e-5));
}

@fragment
fn fs_extract(i: VSOut) -> @location(0) vec4<f32> {
  // Downsample from full-res HDR to half-res by sampling 4 taps.
  let inv = vec2<f32>(p.inv_src_x, p.inv_src_y);

  let c0 = textureSample(t_src, s_src, i.uv + inv * vec2<f32>(-0.5, -0.5)).rgb;
  let c1 = textureSample(t_src, s_src, i.uv + inv * vec2<f32>( 0.5, -0.5)).rgb;
  let c2 = textureSample(t_src, s_src, i.uv + inv * vec2<f32>(-0.5,  0.5)).rgb;
  let c3 = textureSample(t_src, s_src, i.uv + inv * vec2<f32>( 0.5,  0.5)).rgb;

  let avg = (c0 + c1 + c2 + c3) * 0.25;
  let bright = soft_threshold(avg, p.threshold, p.knee);
  return vec4<f32>(bright, 1.0);
}

// 9-tap separable gaussian (good film bloom feel, still fast)
@fragment
fn fs_blur(i: VSOut) -> @location(0) vec4<f32> {
  let dir = vec2<f32>(p.dir_x, p.dir_y);
  let inv = vec2<f32>(p.inv_src_x, p.inv_src_y);
  let step_uv = dir * inv;

  // weights roughly gaussian sigma~2
  let w0 = 0.227027;
  let w1 = 0.1945946;
  let w2 = 0.1216216;
  let w3 = 0.054054;
  let w4 = 0.016216;

  var col = textureSample(t_src, s_src, i.uv).rgb * w0;
  col += textureSample(t_src, s_src, i.uv + step_uv * 1.0).rgb * w1;
  col += textureSample(t_src, s_src, i.uv - step_uv * 1.0).rgb * w1;
  col += textureSample(t_src, s_src, i.uv + step_uv * 2.0).rgb * w2;
  col += textureSample(t_src, s_src, i.uv - step_uv * 2.0).rgb * w2;
  col += textureSample(t_src, s_src, i.uv + step_uv * 3.0).rgb * w3;
  col += textureSample(t_src, s_src, i.uv - step_uv * 3.0).rgb * w3;
  col += textureSample(t_src, s_src, i.uv + step_uv * 4.0).rgb * w4;
  col += textureSample(t_src, s_src, i.uv - step_uv * 4.0).rgb * w4;

  return vec4<f32>(col, 1.0);
}

// ACES (Narkowicz)
fn aces(x: vec3<f32>) -> vec3<f32> {
  let a = 2.51;
  let b = 0.03;
  let c = 2.43;
  let d = 0.59;
  let e = 0.14;
  return clamp((x * (a * x + vec3<f32>(b))) / (x * (c * x + vec3<f32>(d)) + vec3<f32>(e)),
               vec3<f32>(0.0), vec3<f32>(1.0));
}

fn hash12(p: vec2<f32>) -> f32 {
  return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

@group(1) @binding(0) var t_bloom: texture_2d<f32>;
@group(1) @binding(1) var s_bloom: sampler;

@group(2) @binding(0) var<uniform> p2: BloomParams;

@fragment
fn fs_composite(i: VSOut) -> @location(0) vec4<f32> {
  let hdr = textureSample(t_src, s_src, i.uv).rgb;
  let bloom = textureSample(t_bloom, s_bloom, i.uv).rgb * p2.intensity;

  // filmic add
  var col = hdr + bloom;

  // slight vignette
  let aspect = max(scene.res_x, 1.0) / max(scene.res_y, 1.0);
  let pp = vec2<f32>((i.uv.x - 0.5) * aspect, i.uv.y - 0.5);
  let r = length(pp);
  let vig = smoothstep(1.05, 0.25, r);
  col *= 0.65 + 0.35 * vig;

  // tonemap (output linear; do NOT gamma here—swapchain sRGB will handle if needed)
  col = aces(col);

  // subtle film grain after tonemap
  let res = vec2<f32>(max(scene.res_x, 1.0), max(scene.res_y, 1.0));
  let g = (hash12(i.uv * res + vec2<f32>(scene.time * 60.0, scene.frame_seed * 13.0)) - 0.5) * 0.02;
  col += vec3<f32>(g);

  return vec4<f32>(col, 1.0);
}
"#;
