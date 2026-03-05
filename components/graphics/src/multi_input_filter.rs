//! Multi-input GPU filters built for modern GPUs.
//!
//! These filters intentionally avoid legacy Core Image-style APIs and expose
//! explicit, typed operations for common multi-texture workflows.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;

use crate::filter_view::{FilterContext, FilterInput, FilterOutput, GpuFilter};

const MAX_AUX_IMAGES: usize = 3;
const MAX_PARAMS: usize = 8;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct MultiInputUniform {
    output_size: [f32; 2],
    _pad0: [f32; 2],
    op0: [f32; 4],
    op1: [f32; 4],
    op2: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct FilterImage {
    width: u32,
    height: u32,
    rgba8: Arc<[u8]>,
}

impl FilterImage {
    #[must_use]
    pub fn from_rgba8(width: u32, height: u32, rgba8: Vec<u8>) -> Self {
        let expected_len = width as usize * height as usize * 4;
        assert_eq!(
            rgba8.len(),
            expected_len,
            "FilterImage::from_rgba8: expected {expected_len} bytes for {width}x{height} RGBA8 image, got {}",
            rgba8.len()
        );
        Self {
            width,
            height,
            rgba8: Arc::from(rgba8),
        }
    }

    pub fn from_encoded(bytes: &[u8]) -> Result<Self, image::ImageError> {
        let decoded = image::load_from_memory(bytes)?;
        Ok(Self::from_dynamic_image(decoded))
    }

    #[must_use]
    pub fn from_dynamic_image(image: image::DynamicImage) -> Self {
        let rgba = image.to_rgba8();
        let width = rgba.width();
        let height = rgba.height();
        Self {
            width,
            height,
            rgba8: Arc::from(rgba.into_raw()),
        }
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    fn bytes(&self) -> &[u8] {
        self.rgba8.as_ref()
    }
}

#[derive(Clone, Debug)]
pub struct LutImage {
    image: FilterImage,
    size: u32,
}

impl LutImage {
    #[must_use]
    pub fn new(image: FilterImage, size: u32) -> Self {
        assert!(
            size >= 2,
            "LutImage::new: lut size must be >= 2, got {size}"
        );
        let expected_width = size * size;
        assert_eq!(
            image.width(),
            expected_width,
            "LutImage::new: expected width {expected_width} for size {size}, got {}",
            image.width()
        );
        assert_eq!(
            image.height(),
            size,
            "LutImage::new: expected height {size} for size {size}, got {}",
            image.height()
        );
        Self { image, size }
    }

    #[must_use]
    pub fn from_rgba8(size: u32, rgba8: Vec<u8>) -> Self {
        let image = FilterImage::from_rgba8(size * size, size, rgba8);
        Self::new(image, size)
    }

    pub fn from_encoded(size: u32, encoded: &[u8]) -> Result<Self, image::ImageError> {
        let image = FilterImage::from_encoded(encoded)?;
        Ok(Self::new(image, size))
    }

    #[must_use]
    pub const fn size(&self) -> u32 {
        self.size
    }

    fn image(&self) -> &FilterImage {
        &self.image
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
}

impl BlendMode {
    const fn token(self) -> f32 {
        match self {
            Self::Normal => 0.0,
            Self::Multiply => 1.0,
            Self::Screen => 2.0,
            Self::Overlay => 3.0,
        }
    }
}

trait MultiInputOperation: 'static {
    const MODE_ID: u32;
    const AUX_IMAGE_COUNT: usize;

    fn aux_image(&self, index: usize) -> &FilterImage;
    fn write_params(&self, params: &mut [f32; MAX_PARAMS]);
}

#[derive(Debug)]
struct UploadedAuxImage {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

#[derive(Debug)]
struct MultiInputRuntime {
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    pipeline: Option<wgpu::RenderPipeline>,
    sampler: Option<wgpu::Sampler>,
    uniform_buffer: Option<wgpu::Buffer>,
    last_uniform: Option<MultiInputUniform>,
    uploaded_aux_images: [Option<UploadedAuxImage>; MAX_AUX_IMAGES],
    fallback_aux: Option<UploadedAuxImage>,
    setup_error: Option<&'static str>,
}

impl Default for MultiInputRuntime {
    fn default() -> Self {
        Self {
            bind_group_layout: None,
            pipeline: None,
            sampler: None,
            uniform_buffer: None,
            last_uniform: None,
            uploaded_aux_images: [None, None, None],
            fallback_aux: None,
            setup_error: None,
        }
    }
}

pub struct MultiInputFilter<O: MultiInputOperation> {
    operation: O,
    runtime: MultiInputRuntime,
}

impl<O: MultiInputOperation> core::fmt::Debug for MultiInputFilter<O> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MultiInputFilter").finish_non_exhaustive()
    }
}

impl<O: MultiInputOperation> MultiInputFilter<O> {
    #[must_use]
    pub fn new(operation: O) -> Self {
        Self {
            operation,
            runtime: MultiInputRuntime::default(),
        }
    }

    fn set_setup_error(&mut self, err: &'static str) {
        if self.runtime.setup_error.is_none() {
            self.runtime.setup_error = Some(err);
            tracing::error!("[Filter] multi-input setup failed fast: {err}");
        }
    }

    fn has_setup_error(&self) -> bool {
        self.runtime.setup_error.is_some()
    }

    fn create_aux_image_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &FilterImage,
        label: &'static str,
    ) -> UploadedAuxImage {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: image.width(),
                height: image.height(),
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
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            image.bytes(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width() * 4),
                rows_per_image: Some(image.height()),
            },
            wgpu::Extent3d {
                width: image.width(),
                height: image.height(),
                depth_or_array_layers: 1,
            },
        );

        UploadedAuxImage {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            _texture: texture,
        }
    }

    fn create_pipeline(
        ctx: &FilterContext,
    ) -> (
        wgpu::RenderPipeline,
        wgpu::BindGroupLayout,
        wgpu::Sampler,
        wgpu::Buffer,
    ) {
        let shader = crate::shared_context::create_cached_shader_module(
            ctx.device,
            "multi-input filter shader",
            include_str!("shaders/multi_input_filter.wgsl"),
        );

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("multi-input filter bind group layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 4,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 5,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("multi-input filter pipeline layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("multi-input filter pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: shader.as_ref(),
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader.as_ref(),
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.output_format,
                        blend: None,
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
                cache: ctx.pipeline_cache,
            });

        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("multi-input filter sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let uniform_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("multi-input filter uniform buffer"),
            size: core::mem::size_of::<MultiInputUniform>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        (pipeline, bind_group_layout, sampler, uniform_buffer)
    }

    fn encode_uniform(
        mode: u32,
        width: u32,
        height: u32,
        params: [f32; MAX_PARAMS],
    ) -> MultiInputUniform {
        MultiInputUniform {
            output_size: [width as f32, height as f32],
            _pad0: [0.0, 0.0],
            op0: [mode as f32, params[0], params[1], params[2]],
            op1: [params[3], params[4], params[5], params[6]],
            op2: [params[7], 0.0, 0.0, 0.0],
        }
    }
}

impl<O: MultiInputOperation> GpuFilter for MultiInputFilter<O> {
    fn setup(&mut self, ctx: &FilterContext) -> impl Future<Output = ()> {
        if O::AUX_IMAGE_COUNT > MAX_AUX_IMAGES {
            self.set_setup_error("multi-input filter declared too many auxiliary images");
            return core::future::ready(());
        }

        let (pipeline, bind_group_layout, sampler, uniform_buffer) = Self::create_pipeline(ctx);
        self.runtime.pipeline = Some(pipeline);
        self.runtime.bind_group_layout = Some(bind_group_layout);
        self.runtime.sampler = Some(sampler);
        self.runtime.uniform_buffer = Some(uniform_buffer);
        self.runtime.last_uniform = None;

        self.runtime.fallback_aux = Some(Self::create_aux_image_texture(
            ctx.device,
            ctx.queue,
            &FilterImage::from_rgba8(1, 1, vec![0, 0, 0, 255]),
            "multi-input fallback aux",
        ));

        for slot in 0..MAX_AUX_IMAGES {
            if slot < O::AUX_IMAGE_COUNT {
                self.runtime.uploaded_aux_images[slot] = Some(Self::create_aux_image_texture(
                    ctx.device,
                    ctx.queue,
                    self.operation.aux_image(slot),
                    "multi-input aux image",
                ));
            } else {
                self.runtime.uploaded_aux_images[slot] = None;
            }
        }

        core::future::ready(())
    }

    fn render(&mut self, input: &FilterInput, output: &FilterOutput) -> bool {
        if self.has_setup_error() {
            return false;
        }

        let Some(pipeline) = self.runtime.pipeline.as_ref() else {
            return false;
        };
        let Some(bind_group_layout) = self.runtime.bind_group_layout.as_ref() else {
            return false;
        };
        let Some(sampler) = self.runtime.sampler.as_ref() else {
            return false;
        };
        let Some(uniform_buffer) = self.runtime.uniform_buffer.as_ref() else {
            return false;
        };
        let Some(fallback_aux) = self.runtime.fallback_aux.as_ref() else {
            return false;
        };

        let mut params = [0.0f32; MAX_PARAMS];
        self.operation.write_params(&mut params);
        let uniform = Self::encode_uniform(O::MODE_ID, output.width, output.height, params);
        if self.runtime.last_uniform != Some(uniform) {
            input
                .queue
                .write_buffer(uniform_buffer, 0, bytemuck::bytes_of(&uniform));
            self.runtime.last_uniform = Some(uniform);
        }

        let aux0 = self.runtime.uploaded_aux_images[0]
            .as_ref()
            .map_or(&fallback_aux.view, |value| &value.view);
        let aux1 = self.runtime.uploaded_aux_images[1]
            .as_ref()
            .map_or(&fallback_aux.view, |value| &value.view);
        let aux2 = self.runtime.uploaded_aux_images[2]
            .as_ref()
            .map_or(&fallback_aux.view, |value| &value.view);

        let bind_group = input.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("multi-input filter bind group"),
            layout: bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(aux0),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(aux1),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(aux2),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut encoder = input
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("multi-input filter encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("multi-input filter pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }

        input.queue.submit([encoder.finish()]);
        false
    }
}

#[derive(Debug, Clone)]
pub struct BlendWithImage {
    pub image: FilterImage,
    pub amount: f32,
    pub mode: BlendMode,
}

impl MultiInputOperation for BlendWithImage {
    const MODE_ID: u32 = 0;
    const AUX_IMAGE_COUNT: usize = 1;

    fn aux_image(&self, index: usize) -> &FilterImage {
        match index {
            0 => &self.image,
            _ => panic!("BlendWithImage: invalid aux index {index}"),
        }
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.mode.token();
        params[1] = self.amount;
    }
}

#[derive(Debug, Clone)]
pub struct MaskedBlur {
    pub mask: FilterImage,
    pub radius: f32,
    pub strength: f32,
}

impl MultiInputOperation for MaskedBlur {
    const MODE_ID: u32 = 1;
    const AUX_IMAGE_COUNT: usize = 1;

    fn aux_image(&self, index: usize) -> &FilterImage {
        match index {
            0 => &self.mask,
            _ => panic!("MaskedBlur: invalid aux index {index}"),
        }
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.radius;
        params[1] = self.strength;
    }
}

#[derive(Debug, Clone)]
pub struct TransitionToImage {
    pub target: FilterImage,
    pub progress: f32,
    pub softness: f32,
}

impl MultiInputOperation for TransitionToImage {
    const MODE_ID: u32 = 2;
    const AUX_IMAGE_COUNT: usize = 1;

    fn aux_image(&self, index: usize) -> &FilterImage {
        match index {
            0 => &self.target,
            _ => panic!("TransitionToImage: invalid aux index {index}"),
        }
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.progress;
        params[1] = self.softness;
    }
}

#[derive(Debug, Clone)]
pub struct DisplacementWarp {
    pub map: FilterImage,
    pub scale_x: f32,
    pub scale_y: f32,
}

impl MultiInputOperation for DisplacementWarp {
    const MODE_ID: u32 = 3;
    const AUX_IMAGE_COUNT: usize = 1;

    fn aux_image(&self, index: usize) -> &FilterImage {
        match index {
            0 => &self.map,
            _ => panic!("DisplacementWarp: invalid aux index {index}"),
        }
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.scale_x;
        params[1] = self.scale_y;
    }
}

#[derive(Debug, Clone)]
pub struct GuidedSmooth {
    pub guide: FilterImage,
    pub radius: f32,
    pub range_sigma: f32,
    pub amount: f32,
}

impl MultiInputOperation for GuidedSmooth {
    const MODE_ID: u32 = 4;
    const AUX_IMAGE_COUNT: usize = 1;

    fn aux_image(&self, index: usize) -> &FilterImage {
        match index {
            0 => &self.guide,
            _ => panic!("GuidedSmooth: invalid aux index {index}"),
        }
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.radius;
        params[1] = self.range_sigma;
        params[2] = self.amount;
    }
}

#[derive(Debug, Clone)]
pub struct DepthAwareBlur {
    pub depth: FilterImage,
    pub focus_depth: f32,
    pub aperture: f32,
    pub max_radius: f32,
}

impl MultiInputOperation for DepthAwareBlur {
    const MODE_ID: u32 = 5;
    const AUX_IMAGE_COUNT: usize = 1;

    fn aux_image(&self, index: usize) -> &FilterImage {
        match index {
            0 => &self.depth,
            _ => panic!("DepthAwareBlur: invalid aux index {index}"),
        }
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.focus_depth;
        params[1] = self.aperture;
        params[2] = self.max_radius;
    }
}

#[derive(Debug, Clone)]
pub struct TemporalDenoise {
    pub history: FilterImage,
    pub motion: FilterImage,
    pub history_weight: f32,
}

impl MultiInputOperation for TemporalDenoise {
    const MODE_ID: u32 = 6;
    const AUX_IMAGE_COUNT: usize = 2;

    fn aux_image(&self, index: usize) -> &FilterImage {
        match index {
            0 => &self.history,
            1 => &self.motion,
            _ => panic!("TemporalDenoise: invalid aux index {index}"),
        }
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.history_weight;
    }
}

#[derive(Debug, Clone)]
pub struct BackgroundReplace {
    pub matte: FilterImage,
    pub background: FilterImage,
    pub edge_softness: f32,
}

impl MultiInputOperation for BackgroundReplace {
    const MODE_ID: u32 = 7;
    const AUX_IMAGE_COUNT: usize = 2;

    fn aux_image(&self, index: usize) -> &FilterImage {
        match index {
            0 => &self.matte,
            1 => &self.background,
            _ => panic!("BackgroundReplace: invalid aux index {index}"),
        }
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.edge_softness;
    }
}

#[derive(Debug, Clone)]
pub struct LutColorGrade {
    pub lut: LutImage,
    pub intensity: f32,
}

impl MultiInputOperation for LutColorGrade {
    const MODE_ID: u32 = 8;
    const AUX_IMAGE_COUNT: usize = 1;

    fn aux_image(&self, index: usize) -> &FilterImage {
        match index {
            0 => self.lut.image(),
            _ => panic!("LutColorGrade: invalid aux index {index}"),
        }
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.lut.size() as f32;
        params[1] = self.intensity;
    }
}

#[derive(Debug, Clone)]
pub struct ToneCurve {
    pub shadows: f32,
    pub midtones: f32,
    pub highlights: f32,
    pub gamma: f32,
    pub amount: f32,
}

impl MultiInputOperation for ToneCurve {
    const MODE_ID: u32 = 9;
    const AUX_IMAGE_COUNT: usize = 0;

    fn aux_image(&self, index: usize) -> &FilterImage {
        panic!("ToneCurve: no auxiliary image available, requested index {index}")
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.shadows;
        params[1] = self.midtones;
        params[2] = self.highlights;
        params[3] = self.gamma;
        params[4] = self.amount;
    }
}

pub type BlendWithImageFilter = MultiInputFilter<BlendWithImage>;
pub type MaskedBlurFilter = MultiInputFilter<MaskedBlur>;
pub type TransitionToImageFilter = MultiInputFilter<TransitionToImage>;
pub type DisplacementWarpFilter = MultiInputFilter<DisplacementWarp>;
pub type GuidedSmoothFilter = MultiInputFilter<GuidedSmooth>;
pub type DepthAwareBlurFilter = MultiInputFilter<DepthAwareBlur>;
pub type TemporalDenoiseFilter = MultiInputFilter<TemporalDenoise>;
pub type BackgroundReplaceFilter = MultiInputFilter<BackgroundReplace>;
pub type LutColorGradeFilter = MultiInputFilter<LutColorGrade>;
pub type ToneCurveFilter = MultiInputFilter<ToneCurve>;

#[must_use]
pub fn blend_with_image_filter(
    image: FilterImage,
    amount: f32,
    mode: BlendMode,
) -> BlendWithImageFilter {
    MultiInputFilter::new(BlendWithImage {
        image,
        amount,
        mode,
    })
}

#[must_use]
pub fn masked_blur_filter(mask: FilterImage, radius: f32, strength: f32) -> MaskedBlurFilter {
    MultiInputFilter::new(MaskedBlur {
        mask,
        radius,
        strength,
    })
}

#[must_use]
pub fn transition_to_image_filter(
    target: FilterImage,
    progress: f32,
    softness: f32,
) -> TransitionToImageFilter {
    MultiInputFilter::new(TransitionToImage {
        target,
        progress,
        softness,
    })
}

#[must_use]
pub fn displacement_warp_filter(
    map: FilterImage,
    scale_x: f32,
    scale_y: f32,
) -> DisplacementWarpFilter {
    MultiInputFilter::new(DisplacementWarp {
        map,
        scale_x,
        scale_y,
    })
}

#[must_use]
pub fn guided_smooth_filter(
    guide: FilterImage,
    radius: f32,
    range_sigma: f32,
    amount: f32,
) -> GuidedSmoothFilter {
    MultiInputFilter::new(GuidedSmooth {
        guide,
        radius,
        range_sigma,
        amount,
    })
}

#[must_use]
pub fn depth_aware_blur_filter(
    depth: FilterImage,
    focus_depth: f32,
    aperture: f32,
    max_radius: f32,
) -> DepthAwareBlurFilter {
    MultiInputFilter::new(DepthAwareBlur {
        depth,
        focus_depth,
        aperture,
        max_radius,
    })
}

#[must_use]
pub fn temporal_denoise_filter(
    history: FilterImage,
    motion: FilterImage,
    history_weight: f32,
) -> TemporalDenoiseFilter {
    MultiInputFilter::new(TemporalDenoise {
        history,
        motion,
        history_weight,
    })
}

#[must_use]
pub fn background_replace_filter(
    matte: FilterImage,
    background: FilterImage,
    edge_softness: f32,
) -> BackgroundReplaceFilter {
    MultiInputFilter::new(BackgroundReplace {
        matte,
        background,
        edge_softness,
    })
}

#[must_use]
pub fn lut_color_grade_filter(lut: LutImage, intensity: f32) -> LutColorGradeFilter {
    MultiInputFilter::new(LutColorGrade { lut, intensity })
}

#[must_use]
pub fn tone_curve_filter(
    shadows: f32,
    midtones: f32,
    highlights: f32,
    gamma: f32,
    amount: f32,
) -> ToneCurveFilter {
    MultiInputFilter::new(ToneCurve {
        shadows,
        midtones,
        highlights,
        gamma,
        amount,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_image_rejects_invalid_buffer_len() {
        let result = std::panic::catch_unwind(|| FilterImage::from_rgba8(2, 2, vec![0; 3]));
        assert!(result.is_err());
    }

    #[test]
    fn blend_mode_tokens_are_stable() {
        assert_eq!(BlendMode::Normal.token(), 0.0);
        assert_eq!(BlendMode::Multiply.token(), 1.0);
        assert_eq!(BlendMode::Screen.token(), 2.0);
        assert_eq!(BlendMode::Overlay.token(), 3.0);
    }

    #[test]
    fn lut_image_rejects_invalid_dimensions() {
        let bad_image = FilterImage::from_rgba8(16, 15, vec![0; 16 * 15 * 4]);
        let result = std::panic::catch_unwind(|| LutImage::new(bad_image, 4));
        assert!(result.is_err());
    }
}
