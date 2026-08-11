//! Multi-input GPU filters built for modern GPUs.
//!
//! These filters intentionally avoid legacy Core Image-style APIs and expose
//! explicit, typed operations for common multi-texture workflows.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::future::Future;
use num_traits::ToPrimitive;

use crate::{Effect, EffectContext, EffectInput, EffectOutput};

const MAX_AUX_IMAGES: usize = 2;
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

/// Immutable RGBA8 image payload uploaded as an auxiliary filter input.
#[derive(Clone, Debug)]
pub struct FilterImage {
    width: u32,
    height: u32,
    rgba8: Arc<[u8]>,
}

impl FilterImage {
    /// Creates an RGBA8 filter image from raw bytes.
    ///
    /// # Panics
    ///
    /// Panics when `rgba8` does not contain exactly `width * height * 4` bytes.
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

    /// Decodes an encoded image and converts it to RGBA8 pixels.
    ///
    /// # Errors
    ///
    /// Returns the decode error when the bytes cannot be parsed as an image.
    pub fn from_encoded(bytes: &[u8]) -> Result<Self, image::ImageError> {
        let decoded = image::load_from_memory(bytes)?;
        Ok(Self::from_dynamic_image(&decoded))
    }

    /// Converts a dynamic image into a filter image.
    #[must_use]
    pub fn from_dynamic_image(image: &image::DynamicImage) -> Self {
        let rgba = image.to_rgba8();
        let width = rgba.width();
        let height = rgba.height();
        Self {
            width,
            height,
            rgba8: Arc::from(rgba.into_raw()),
        }
    }

    /// Returns the image width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the image height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    fn bytes(&self) -> &[u8] {
        self.rgba8.as_ref()
    }
}

/// A 3D LUT packed into the common 2D strip layout.
#[derive(Clone, Debug)]
pub struct LutImage {
    image: FilterImage,
    size: u32,
}

impl LutImage {
    /// Creates a LUT image from an already validated filter image.
    ///
    /// # Panics
    ///
    /// Panics when `size < 2` or when `image` does not match the expected strip dimensions.
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

    /// Creates a LUT image from RGBA8 bytes.
    #[must_use]
    pub fn from_rgba8(size: u32, rgba8: Vec<u8>) -> Self {
        let image = FilterImage::from_rgba8(size * size, size, rgba8);
        Self::new(image, size)
    }

    /// Decodes a LUT image from encoded image bytes.
    ///
    /// # Errors
    ///
    /// Returns the decode error when the bytes cannot be parsed as an image.
    pub fn from_encoded(size: u32, encoded: &[u8]) -> Result<Self, image::ImageError> {
        let image = FilterImage::from_encoded(encoded)?;
        Ok(Self::new(image, size))
    }

    /// Returns the LUT cube size.
    #[must_use]
    pub const fn size(&self) -> u32 {
        self.size
    }

    const fn image(&self) -> &FilterImage {
        &self.image
    }
}

/// Blend operators for combining the input image with an auxiliary image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    /// Alpha compositing with the auxiliary image.
    Normal,
    /// Multiply input and auxiliary colors.
    Multiply,
    /// Screen the auxiliary image over the input.
    Screen,
    /// Overlay the auxiliary image onto the input.
    Overlay,
    /// Keep the darker channel from each source.
    Darken,
    /// Keep the lighter channel from each source.
    Lighten,
    /// Apply a soft light blend.
    SoftLight,
    /// Apply a hard light blend.
    HardLight,
    /// Subtract shared luminance to emphasize differences.
    Difference,
    /// Blend using the exclusion operator.
    Exclusion,
    /// Brighten the input with color dodge.
    ColorDodge,
    /// Darken the input with color burn.
    ColorBurn,
    /// Hue from the auxiliary, saturation and luminance from the input.
    Hue,
    /// Saturation from the auxiliary, hue and luminance from the input.
    Saturation,
    /// Hue and saturation from the auxiliary, luminance from the input.
    Color,
    /// Luminance from the auxiliary, hue and saturation from the input.
    Luminosity,
}

/// Swipe direction for image transition filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionDirection {
    /// Reveal from left to right.
    LeftToRight,
    /// Reveal from right to left.
    RightToLeft,
    /// Reveal from top to bottom.
    TopToBottom,
    /// Reveal from bottom to top.
    BottomToTop,
}

impl TransitionDirection {
    const fn token(self) -> f32 {
        match self {
            Self::LeftToRight => 0.0,
            Self::RightToLeft => 1.0,
            Self::TopToBottom => 2.0,
            Self::BottomToTop => 3.0,
        }
    }
}

impl BlendMode {
    const fn token(self) -> f32 {
        match self {
            Self::Normal => 0.0,
            Self::Multiply => 1.0,
            Self::Screen => 2.0,
            Self::Overlay => 3.0,
            Self::Darken => 4.0,
            Self::Lighten => 5.0,
            Self::SoftLight => 6.0,
            Self::HardLight => 7.0,
            Self::Difference => 8.0,
            Self::Exclusion => 9.0,
            Self::ColorDodge => 10.0,
            Self::ColorBurn => 11.0,
            Self::Hue => 12.0,
            Self::Saturation => 13.0,
            Self::Color => 14.0,
            Self::Luminosity => 15.0,
        }
    }
}

/// Low-level operation contract implemented by concrete multi-input filters.
pub trait MultiInputOperation: 'static {
    /// Shader mode selector used by the shared multi-input shader.
    const MODE_ID: u32;
    /// Number of auxiliary images required by the operation.
    const AUX_IMAGE_COUNT: usize;

    /// Returns the auxiliary image bound at the given slot.
    fn aux_image(&self, index: usize) -> &FilterImage;
    /// Writes operation-specific parameters into the shared uniform buffer.
    fn write_params(&self, params: &mut [f32; MAX_PARAMS]);
}

#[derive(Debug)]
struct UploadedAuxImage {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Bind group reused across frames; only the captured input view varies
/// between renders, so the cache is keyed on that view's identity.
#[derive(Debug)]
struct CachedBindGroup {
    input_view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

#[derive(Debug, Default)]
struct MultiInputRuntime {
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    pipeline: Option<wgpu::RenderPipeline>,
    sampler: Option<wgpu::Sampler>,
    uniform_buffer: Option<wgpu::Buffer>,
    last_uniform: Option<MultiInputUniform>,
    uploaded_aux_images: [Option<UploadedAuxImage>; MAX_AUX_IMAGES],
    fallback_aux: Option<UploadedAuxImage>,
    cached_bind_group: Option<CachedBindGroup>,
    setup_error: Option<crate::EffectSetupError>,
}

impl MultiInputRuntime {
    /// Uploads the uniform if it changed and (re)creates the cached bind
    /// group when the input view rotated. Fails fast when any resource that
    /// setup should have produced is missing.
    fn ensure_bind_group(
        &mut self,
        input: &EffectInput,
        uniform: MultiInputUniform,
    ) -> Result<(), crate::EffectRenderError> {
        let Some(bind_group_layout) = self.bind_group_layout.as_ref() else {
            return Err(crate::EffectRenderError::MissingResource(
                "multi-input filter bind group layout missing after setup",
            ));
        };
        let Some(sampler) = self.sampler.as_ref() else {
            return Err(crate::EffectRenderError::MissingResource(
                "multi-input filter sampler missing after setup",
            ));
        };
        let Some(uniform_buffer) = self.uniform_buffer.as_ref() else {
            return Err(crate::EffectRenderError::MissingResource(
                "multi-input filter uniform buffer missing after setup",
            ));
        };
        let Some(fallback_aux) = self.fallback_aux.as_ref() else {
            return Err(crate::EffectRenderError::MissingResource(
                "multi-input filter fallback auxiliary texture missing after setup",
            ));
        };

        if self.last_uniform != Some(uniform) {
            input
                .queue
                .write_buffer(uniform_buffer, 0, bytemuck::bytes_of(&uniform));
            self.last_uniform = Some(uniform);
        }

        if self
            .cached_bind_group
            .as_ref()
            .is_none_or(|cached| cached.input_view != input.view)
        {
            let aux_views: [&wgpu::TextureView; MAX_AUX_IMAGES] = core::array::from_fn(|slot| {
                self.uploaded_aux_images[slot]
                    .as_ref()
                    .map_or(&fallback_aux.view, |value| &value.view)
            });
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
                        resource: wgpu::BindingResource::TextureView(aux_views[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(aux_views[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                ],
            });
            self.cached_bind_group = Some(CachedBindGroup {
                input_view: input.view.clone(),
                bind_group,
            });
        }
        Ok(())
    }
}

/// Generic GPU filter wrapper for operations that need multiple input textures.
pub struct MultiInputFilter<O: MultiInputOperation> {
    operation: O,
    runtime: MultiInputRuntime,
}

impl<O: MultiInputOperation> fmt::Debug for MultiInputFilter<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MultiInputFilter").finish_non_exhaustive()
    }
}

impl<O: MultiInputOperation> MultiInputFilter<O> {
    /// Creates a multi-input filter from a concrete operation.
    #[must_use]
    pub fn new(operation: O) -> Self {
        Self {
            operation,
            runtime: MultiInputRuntime::default(),
        }
    }

    fn set_setup_error(&mut self, err: &crate::EffectSetupError) {
        if self.runtime.setup_error.is_none() {
            self.runtime.setup_error = Some(err.clone());
            tracing::error!("[Filter] multi-input setup failed fast: {err}");
        }
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

    fn create_bind_group_layout(ctx: &EffectContext) -> wgpu::BindGroupLayout {
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
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            })
    }

    fn create_pipeline(
        ctx: &EffectContext,
    ) -> (
        wgpu::RenderPipeline,
        wgpu::BindGroupLayout,
        wgpu::Sampler,
        wgpu::Buffer,
    ) {
        let (vertex_shader, fragment_shader) = crate::compiled_shaders::MULTI_INPUT
            .create_render_stages(ctx.device, "vs_main", "fs_main");
        let bind_group_layout = Self::create_bind_group_layout(ctx);

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("multi-input filter pipeline layout"),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("multi-input filter pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: vertex_shader.module(),
                    entry_point: Some(vertex_shader.entry_point()),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: fragment_shader.module(),
                    entry_point: Some(fragment_shader.entry_point()),
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
                multiview_mask: None,
                cache: None,
            });

        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("multi-input filter sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
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
            output_size: [u32_to_f32(width), u32_to_f32(height)],
            _pad0: [0.0, 0.0],
            op0: [u32_to_f32(mode), params[0], params[1], params[2]],
            op1: [params[3], params[4], params[5], params[6]],
            op2: [params[7], 0.0, 0.0, 0.0],
        }
    }
}

impl<O: MultiInputOperation> Effect for MultiInputFilter<O> {
    fn setup(&mut self, ctx: &EffectContext) -> impl Future<Output = crate::EffectSetupResult> {
        if O::AUX_IMAGE_COUNT > MAX_AUX_IMAGES {
            let err = crate::EffectSetupError::Other(
                "multi-input filter declared too many auxiliary images",
            );
            self.set_setup_error(&err);
            return core::future::ready(Err(err));
        }

        let (pipeline, bind_group_layout, sampler, uniform_buffer) = Self::create_pipeline(ctx);
        self.runtime.pipeline = Some(pipeline);
        self.runtime.bind_group_layout = Some(bind_group_layout);
        self.runtime.sampler = Some(sampler);
        self.runtime.uniform_buffer = Some(uniform_buffer);
        self.runtime.last_uniform = None;
        self.runtime.cached_bind_group = None;

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

        core::future::ready(Ok(()))
    }

    fn encode_render(
        &mut self,
        input: &EffectInput,
        output: &EffectOutput,
        encoder: &mut wgpu::CommandEncoder,
    ) -> crate::EffectRenderResult {
        if let Some(err) = &self.runtime.setup_error {
            return Err(crate::EffectRenderError::SetupFailed(err.clone()));
        }

        let mut params = [0.0f32; MAX_PARAMS];
        self.operation.write_params(&mut params);
        let uniform = Self::encode_uniform(O::MODE_ID, output.width, output.height, params);
        self.runtime.ensure_bind_group(input, uniform)?;
        let Some(pipeline) = self.runtime.pipeline.as_ref() else {
            return Err(crate::EffectRenderError::MissingResource(
                "multi-input filter pipeline missing after setup",
            ));
        };
        let Some(cached) = self.runtime.cached_bind_group.as_ref() else {
            return Err(crate::EffectRenderError::MissingResource(
                "multi-input filter bind group missing after creation",
            ));
        };
        let bind_group = &cached.bind_group;

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
                multiview_mask: None,
            });
            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(0, bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }

        Ok(false)
    }
}

/// Blends the current frame with a second image.
#[derive(Debug, Clone)]
pub struct BlendWithImage {
    /// Auxiliary image used during blending.
    pub image: FilterImage,
    /// Blend strength in the range expected by the shader.
    pub amount: f32,
    /// Blend operator to apply.
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

/// Applies blur intensity based on a mask image.
#[derive(Debug, Clone)]
pub struct MaskedBlur {
    /// Blur mask image.
    pub mask: FilterImage,
    /// Blur radius in pixels.
    pub radius: f32,
    /// Additional blur strength multiplier.
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

/// Transitions from the input image to a target image.
#[derive(Debug, Clone)]
pub struct TransitionToImage {
    /// Target image revealed by the transition.
    pub target: FilterImage,
    /// Transition progress from `0.0` to `1.0`.
    pub progress: f32,
    /// Feathering amount around the transition boundary.
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

/// Warps the input image using a displacement map.
#[derive(Debug, Clone)]
pub struct DisplacementWarp {
    /// Displacement map image.
    pub map: FilterImage,
    /// Horizontal displacement scale.
    pub scale_x: f32,
    /// Vertical displacement scale.
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

/// Smooths the input image with guidance from another image.
#[derive(Debug, Clone)]
pub struct GuidedSmooth {
    /// Guide image that preserves major edges.
    pub guide: FilterImage,
    /// Filter radius.
    pub radius: f32,
    /// Range sensitivity.
    pub range_sigma: f32,
    /// Blend amount for the smoothed result.
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

/// Blurs the input based on a depth texture.
#[derive(Debug, Clone)]
pub struct DepthAwareBlur {
    /// Depth map that drives blur strength.
    pub depth: FilterImage,
    /// Depth plane that remains in focus.
    pub focus_depth: f32,
    /// Simulated aperture size.
    pub aperture: f32,
    /// Maximum blur radius.
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

/// Denoises the current frame using history and motion textures.
#[derive(Debug, Clone)]
pub struct TemporalDenoise {
    /// Previous filtered frame.
    pub history: FilterImage,
    /// Motion vectors or reprojection helper image.
    pub motion: FilterImage,
    /// Weight assigned to history data.
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

/// Composites the subject over a replacement background.
#[derive(Debug, Clone)]
pub struct BackgroundReplace {
    /// Foreground matte image.
    pub matte: FilterImage,
    /// Replacement background image.
    pub background: FilterImage,
    /// Softening factor for matte edges.
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

/// Applies a 3D LUT-based color grade.
#[derive(Debug, Clone)]
pub struct LutColorGrade {
    /// LUT texture encoded as a 2D strip.
    pub lut: LutImage,
    /// Grade intensity multiplier.
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
        params[0] = u32_to_f32(self.lut.size());
        params[1] = self.intensity;
    }
}

/// Shapes tonal regions of the input image.
#[derive(Debug, Clone)]
pub struct ToneCurve {
    /// Shadow adjustment.
    pub shadows: f32,
    /// Midtone adjustment.
    pub midtones: f32,
    /// Highlight adjustment.
    pub highlights: f32,
    /// Gamma adjustment.
    pub gamma: f32,
    /// Overall blend amount.
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

/// Performs a directional swipe transition to a target image.
#[derive(Debug, Clone)]
pub struct SwipeTransitionToImage {
    /// Target image revealed by the transition.
    pub target: FilterImage,
    /// Transition progress from `0.0` to `1.0`.
    pub progress: f32,
    /// Feathering amount around the swipe edge.
    pub softness: f32,
    /// Swipe direction.
    pub direction: TransitionDirection,
}

impl MultiInputOperation for SwipeTransitionToImage {
    const MODE_ID: u32 = 10;
    const AUX_IMAGE_COUNT: usize = 1;

    fn aux_image(&self, index: usize) -> &FilterImage {
        match index {
            0 => &self.target,
            _ => panic!("SwipeTransitionToImage: invalid aux index {index}"),
        }
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.progress;
        params[1] = self.softness;
        params[2] = self.direction.token();
    }
}

/// Performs a radial transition to a target image.
#[derive(Debug, Clone)]
pub struct RadialTransitionToImage {
    /// Target image revealed by the transition.
    pub target: FilterImage,
    /// Transition progress from `0.0` to `1.0`.
    pub progress: f32,
    /// Feathering amount around the radial edge.
    pub softness: f32,
    /// Horizontal transition center in normalized coordinates.
    pub center_x: f32,
    /// Vertical transition center in normalized coordinates.
    pub center_y: f32,
}

impl MultiInputOperation for RadialTransitionToImage {
    const MODE_ID: u32 = 11;
    const AUX_IMAGE_COUNT: usize = 1;

    fn aux_image(&self, index: usize) -> &FilterImage {
        match index {
            0 => &self.target,
            _ => panic!("RadialTransitionToImage: invalid aux index {index}"),
        }
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.progress;
        params[1] = self.softness;
        params[2] = self.center_x;
        params[3] = self.center_y;
    }
}

/// Performs a zoom-based transition to a target image.
#[derive(Debug, Clone)]
pub struct ZoomTransitionToImage {
    /// Target image revealed by the transition.
    pub target: FilterImage,
    /// Transition progress from `0.0` to `1.0`.
    pub progress: f32,
    /// Zoom magnitude applied during the transition.
    pub amount: f32,
    /// Horizontal zoom center in normalized coordinates.
    pub center_x: f32,
    /// Vertical zoom center in normalized coordinates.
    pub center_y: f32,
}

impl MultiInputOperation for ZoomTransitionToImage {
    const MODE_ID: u32 = 12;
    const AUX_IMAGE_COUNT: usize = 1;

    fn aux_image(&self, index: usize) -> &FilterImage {
        match index {
            0 => &self.target,
            _ => panic!("ZoomTransitionToImage: invalid aux index {index}"),
        }
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.progress;
        params[1] = self.amount;
        params[2] = self.center_x;
        params[3] = self.center_y;
    }
}

/// Performs a displacement-driven transition to a target image.
#[derive(Debug, Clone)]
pub struct DisplacementTransitionToImage {
    /// Target image revealed by the transition.
    pub target: FilterImage,
    /// Displacement map used during transition.
    pub map: FilterImage,
    /// Transition progress from `0.0` to `1.0`.
    pub progress: f32,
    /// Displacement strength.
    pub scale: f32,
}

impl MultiInputOperation for DisplacementTransitionToImage {
    const MODE_ID: u32 = 13;
    const AUX_IMAGE_COUNT: usize = 2;

    fn aux_image(&self, index: usize) -> &FilterImage {
        match index {
            0 => &self.target,
            1 => &self.map,
            _ => panic!("DisplacementTransitionToImage: invalid aux index {index}"),
        }
    }

    fn write_params(&self, params: &mut [f32; MAX_PARAMS]) {
        params[0] = self.progress;
        params[1] = self.scale;
    }
}

/// Filter type for [`BlendWithImage`].
pub type BlendWithImageFilter = MultiInputFilter<BlendWithImage>;
/// Filter type for [`MaskedBlur`].
pub type MaskedBlurFilter = MultiInputFilter<MaskedBlur>;
/// Filter type for [`TransitionToImage`].
pub type TransitionToImageFilter = MultiInputFilter<TransitionToImage>;
/// Filter type for [`DisplacementWarp`].
pub type DisplacementWarpFilter = MultiInputFilter<DisplacementWarp>;
/// Filter type for [`GuidedSmooth`].
pub type GuidedSmoothFilter = MultiInputFilter<GuidedSmooth>;
/// Filter type for [`DepthAwareBlur`].
pub type DepthAwareBlurFilter = MultiInputFilter<DepthAwareBlur>;
/// Filter type for [`TemporalDenoise`].
pub type TemporalDenoiseFilter = MultiInputFilter<TemporalDenoise>;
/// Filter type for [`BackgroundReplace`].
pub type BackgroundReplaceFilter = MultiInputFilter<BackgroundReplace>;
/// Filter type for [`LutColorGrade`].
pub type LutColorGradeFilter = MultiInputFilter<LutColorGrade>;
/// Filter type for [`ToneCurve`].
pub type ToneCurveFilter = MultiInputFilter<ToneCurve>;
/// Filter type for [`SwipeTransitionToImage`].
pub type SwipeTransitionToImageFilter = MultiInputFilter<SwipeTransitionToImage>;
/// Filter type for [`RadialTransitionToImage`].
pub type RadialTransitionToImageFilter = MultiInputFilter<RadialTransitionToImage>;
/// Filter type for [`ZoomTransitionToImage`].
pub type ZoomTransitionToImageFilter = MultiInputFilter<ZoomTransitionToImage>;
/// Filter type for [`DisplacementTransitionToImage`].
pub type DisplacementTransitionToImageFilter = MultiInputFilter<DisplacementTransitionToImage>;

#[must_use]
/// Creates a blend filter that composites an auxiliary image over the input.
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
/// Creates a masked blur filter.
pub fn masked_blur_filter(mask: FilterImage, radius: f32, strength: f32) -> MaskedBlurFilter {
    MultiInputFilter::new(MaskedBlur {
        mask,
        radius,
        strength,
    })
}

#[must_use]
/// Creates a single-image transition filter.
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
/// Creates a displacement warp filter.
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
/// Creates a guided smoothing filter.
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
/// Creates a depth-aware blur filter.
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
/// Creates a temporal denoise filter.
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
/// Creates a background replacement filter.
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
/// Creates a LUT color grading filter.
pub fn lut_color_grade_filter(lut: LutImage, intensity: f32) -> LutColorGradeFilter {
    MultiInputFilter::new(LutColorGrade { lut, intensity })
}

#[must_use]
/// Creates a tone-curve adjustment filter.
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

#[must_use]
/// Creates a directional swipe transition filter.
pub fn swipe_transition_to_image_filter(
    target: FilterImage,
    progress: f32,
    softness: f32,
    direction: TransitionDirection,
) -> SwipeTransitionToImageFilter {
    MultiInputFilter::new(SwipeTransitionToImage {
        target,
        progress,
        softness,
        direction,
    })
}

#[must_use]
/// Creates a radial transition filter.
pub fn radial_transition_to_image_filter(
    target: FilterImage,
    progress: f32,
    softness: f32,
    center_x: f32,
    center_y: f32,
) -> RadialTransitionToImageFilter {
    MultiInputFilter::new(RadialTransitionToImage {
        target,
        progress,
        softness,
        center_x,
        center_y,
    })
}

#[must_use]
/// Creates a zoom transition filter.
pub fn zoom_transition_to_image_filter(
    target: FilterImage,
    progress: f32,
    amount: f32,
    center_x: f32,
    center_y: f32,
) -> ZoomTransitionToImageFilter {
    MultiInputFilter::new(ZoomTransitionToImage {
        target,
        progress,
        amount,
        center_x,
        center_y,
    })
}

#[must_use]
/// Creates a displacement-driven transition filter.
pub fn displacement_transition_to_image_filter(
    target: FilterImage,
    map: FilterImage,
    progress: f32,
    scale: f32,
) -> DisplacementTransitionToImageFilter {
    MultiInputFilter::new(DisplacementTransitionToImage {
        target,
        map,
        progress,
        scale,
    })
}

fn u32_to_f32(value: u32) -> f32 {
    value
        .to_f32()
        .expect("multi_input_filter: u32 value must be representable as f32")
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
        assert_eq!(BlendMode::Darken.token(), 4.0);
        assert_eq!(BlendMode::Lighten.token(), 5.0);
        assert_eq!(BlendMode::SoftLight.token(), 6.0);
        assert_eq!(BlendMode::HardLight.token(), 7.0);
        assert_eq!(BlendMode::Difference.token(), 8.0);
        assert_eq!(BlendMode::Exclusion.token(), 9.0);
        assert_eq!(BlendMode::ColorDodge.token(), 10.0);
        assert_eq!(BlendMode::ColorBurn.token(), 11.0);
        assert_eq!(BlendMode::Hue.token(), 12.0);
        assert_eq!(BlendMode::Saturation.token(), 13.0);
        assert_eq!(BlendMode::Color.token(), 14.0);
        assert_eq!(BlendMode::Luminosity.token(), 15.0);
    }

    #[test]
    fn transition_direction_tokens_are_stable() {
        assert_eq!(TransitionDirection::LeftToRight.token(), 0.0);
        assert_eq!(TransitionDirection::RightToLeft.token(), 1.0);
        assert_eq!(TransitionDirection::TopToBottom.token(), 2.0);
        assert_eq!(TransitionDirection::BottomToTop.token(), 3.0);
    }

    #[test]
    fn lut_image_rejects_invalid_dimensions() {
        let bad_image = FilterImage::from_rgba8(16, 15, vec![0; 16 * 15 * 4]);
        let result = std::panic::catch_unwind(|| LutImage::new(bad_image, 4));
        assert!(result.is_err());
    }
}
