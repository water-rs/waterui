//! GPU-accelerated Image view using wgpu.
//!
//! This module provides [`Image`], a View that displays images on the GPU
//! with hardware-accelerated filters. Images are stored as GPU textures,
//! and filters are applied entirely on the GPU using the `filtrate` crate.
//!
//! # Example
//!
//! ```ignore
//! use waterui_media::Image;
//!
//! // Create an image from RGBA pixel data
//! Image::new(rgba_pixels, 800, 600)
//!     .blur(5.0)
//!     .brightness(0.1)
//!     .saturation(1.2)
//! ```

use std::sync::Arc;

use filtrate::{Filter, FilterPipeline};
use waterui_core::{Environment, View};
use waterui_graphics::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};

/// A GPU-accelerated image view.
///
/// `Image` stores pixel data that will be uploaded to a GPU texture when the
/// view is set up. The pixel data is consumed during setup and not retained
/// in memory. Filters are applied on the GPU via compute shaders.
///
/// # Memory Model
///
/// - **Before setup**: Holds pending pixel data (Vec<u8>)
/// - **After setup**: Holds only GPU texture (no CPU pixel data)
///
/// This design ensures efficient memory usage by not duplicating data between
/// CPU and GPU memory.
///
/// # Example
///
/// ```ignore
/// use waterui_media::Image;
///
/// // RGBA pixel data (4 bytes per pixel)
/// let pixels: Vec<u8> = vec![255, 0, 0, 255]; // 1x1 red pixel
///
/// let image = Image::new(pixels, 1, 1)
///     .blur(2.0)
///     .brightness(0.1);
/// ```
#[derive(Debug)]
pub struct Image {
    renderer: ImageRenderer,
}

impl Image {
    /// Creates a new Image from RGBA pixel data.
    ///
    /// The pixel data must be in RGBA format (4 bytes per pixel) and have
    /// exactly `width * height * 4` bytes. The data will be uploaded to a
    /// GPU texture when the view is set up.
    ///
    /// # Arguments
    ///
    /// * `pixels` - RGBA pixel data (4 bytes per pixel)
    /// * `width` - Image width in pixels
    /// * `height` - Image height in pixels
    ///
    /// # Panics
    ///
    /// Panics if the pixel data length doesn't match `width * height * 4`.
    #[must_use]
    pub fn new(pixels: Vec<u8>, width: u32, height: u32) -> Self {
        assert_eq!(
            pixels.len(),
            (width * height * 4) as usize,
            "Pixel data length must be width * height * 4"
        );
        Self {
            renderer: ImageRenderer::new(pixels, width, height),
        }
    }

    /// Apply a gaussian blur filter.
    ///
    /// # Arguments
    ///
    /// * `radius` - Blur radius in pixels (higher = more blur)
    #[must_use]
    pub fn blur(mut self, radius: f32) -> Self {
        self.renderer.filters.push(Filter::Blur { radius });
        self
    }

    /// Adjust brightness.
    ///
    /// # Arguments
    ///
    /// * `amount` - Brightness adjustment (-1.0 = black, 0.0 = unchanged, 1.0 = white)
    #[must_use]
    pub fn brightness(mut self, amount: f32) -> Self {
        self.renderer.filters.push(Filter::Brightness { amount });
        self
    }

    /// Adjust color saturation.
    ///
    /// # Arguments
    ///
    /// * `amount` - Saturation multiplier (0.0 = grayscale, 1.0 = unchanged, >1.0 = more saturated)
    #[must_use]
    pub fn saturation(mut self, amount: f32) -> Self {
        self.renderer.filters.push(Filter::Saturation { amount });
        self
    }

    /// Adjust contrast.
    ///
    /// # Arguments
    ///
    /// * `amount` - Contrast multiplier (0.0 = gray, 1.0 = unchanged, >1.0 = more contrast)
    #[must_use]
    pub fn contrast(mut self, amount: f32) -> Self {
        self.renderer.filters.push(Filter::Contrast { amount });
        self
    }

    /// Convert to grayscale.
    ///
    /// # Arguments
    ///
    /// * `intensity` - Mix factor (0.0 = original, 1.0 = full grayscale)
    #[must_use]
    pub fn grayscale(mut self, intensity: f32) -> Self {
        self.renderer.filters.push(Filter::Grayscale { intensity });
        self
    }

    /// Rotate hue around the color wheel.
    ///
    /// # Arguments
    ///
    /// * `angle` - Rotation angle in degrees (0-360)
    #[must_use]
    pub fn hue_rotate(mut self, angle: f32) -> Self {
        self.renderer.filters.push(Filter::HueRotation { angle });
        self
    }

    /// Invert all colors.
    #[must_use]
    pub fn invert(mut self) -> Self {
        self.renderer.filters.push(Filter::Invert);
        self
    }

    /// Apply sepia tone effect.
    ///
    /// # Arguments
    ///
    /// * `intensity` - Sepia intensity (0.0 = original, 1.0 = full sepia)
    #[must_use]
    pub fn sepia(mut self, intensity: f32) -> Self {
        self.renderer.filters.push(Filter::Sepia { intensity });
        self
    }

    /// Add vignette effect (darkened corners).
    ///
    /// # Arguments
    ///
    /// * `radius` - Inner radius where vignette starts (0.0-1.0)
    /// * `softness` - How soft the vignette edge is (0.0-1.0)
    #[must_use]
    pub fn vignette(mut self, radius: f32, softness: f32) -> Self {
        self.renderer
            .filters
            .push(Filter::Vignette { radius, softness });
        self
    }

    /// Sharpen image details.
    ///
    /// # Arguments
    ///
    /// * `amount` - Sharpening strength (0.0 = unchanged, 1.0 = normal, >1.0 = more sharp)
    #[must_use]
    pub fn sharpen(mut self, amount: f32) -> Self {
        self.renderer.filters.push(Filter::Sharpen { amount });
        self
    }

    /// Apply a custom filter.
    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.renderer.filters.push(filter);
        self
    }

    /// Get the image dimensions (width, height).
    #[must_use]
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.renderer.width, self.renderer.height)
    }

    /// Get the image width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.renderer.width
    }

    /// Get the image height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.renderer.height
    }
}

impl View for Image {
    fn body(self, _env: &Environment) -> impl View {
        GpuSurface::new(self.renderer)
    }
}

/// Internal GPU renderer for images.
struct ImageRenderer {
    /// Pending pixel data (consumed during setup)
    pending_pixels: Option<Vec<u8>>,
    /// Image width
    width: u32,
    /// Image height
    height: u32,
    /// GPU texture (created during setup)
    texture: Option<wgpu::Texture>,
    /// Filter pipeline (created during setup)
    filter_pipeline: Option<FilterPipeline>,
    /// Render pipeline for displaying to screen
    render_pipeline: Option<wgpu::RenderPipeline>,
    /// Bind group for rendering
    bind_group: Option<wgpu::BindGroup>,
    /// Sampler for texture sampling
    sampler: Option<wgpu::Sampler>,
    /// Filters to apply
    filters: Vec<Filter>,
    /// Filtered texture (output of filter pipeline)
    filtered_texture: Option<wgpu::Texture>,
    /// Flag indicating filters have been applied
    filters_applied: bool,
}

impl core::fmt::Debug for ImageRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ImageRenderer")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("filters", &self.filters)
            .finish_non_exhaustive()
    }
}

impl ImageRenderer {
    fn new(pixels: Vec<u8>, width: u32, height: u32) -> Self {
        Self {
            pending_pixels: Some(pixels),
            width,
            height,
            texture: None,
            filter_pipeline: None,
            render_pipeline: None,
            bind_group: None,
            sampler: None,
            filters: Vec::new(),
            filtered_texture: None,
            filters_applied: false,
        }
    }

    fn create_render_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
        // Simple shader to render a texture to the screen
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Image render shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/image_render.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Image bind group layout"),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Image pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Image render pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Image sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        (render_pipeline, bind_group_layout, sampler)
    }
}

impl GpuRenderer for ImageRenderer {
    fn setup(&mut self, ctx: &GpuContext) {
        tracing::debug!(
            "[ImageRenderer] setup() called with format: {:?}, size: {}x{}",
            ctx.surface_format,
            self.width,
            self.height
        );

        // Upload pending pixels to GPU texture
        if let Some(pixels) = self.pending_pixels.take() {
            let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Image source texture"),
                size: wgpu::Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            });

            ctx.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.width * 4),
                    rows_per_image: Some(self.height),
                },
                wgpu::Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
            );

            self.texture = Some(texture);
            // pixels dropped here - NOT stored
        }

        // Create filter pipeline
        self.filter_pipeline = Some(FilterPipeline::new(
            Arc::new(ctx.device.clone()),
            Arc::new(ctx.queue.clone()),
        ));

        // Create filtered texture for filter output
        if !self.filters.is_empty() {
            self.filtered_texture = Some(ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Image filtered texture"),
                size: wgpu::Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }));
        }

        // Create render pipeline
        let (render_pipeline, bind_group_layout, sampler) =
            Self::create_render_pipeline(ctx.device, ctx.surface_format);

        // Determine which texture to use for rendering
        let display_texture = self
            .filtered_texture
            .as_ref()
            .or(self.texture.as_ref())
            .expect("Texture should be created");

        let texture_view = display_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Image bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        self.render_pipeline = Some(render_pipeline);
        self.bind_group = Some(bind_group);
        self.sampler = Some(sampler);
    }

    fn render(&mut self, frame: &GpuFrame) {
        tracing::debug!(
            "[ImageRenderer] render() called, format: {:?}, size: {}x{}, has_pipeline: {}",
            frame.format,
            frame.width,
            frame.height,
            self.render_pipeline.is_some()
        );

        // Apply filters once if not already applied
        if !self.filters_applied && !self.filters.is_empty() {
            if let (Some(pipeline), Some(src), Some(dst)) = (
                &self.filter_pipeline,
                &self.texture,
                &self.filtered_texture,
            ) {
                pipeline.apply(src, dst, &self.filters);
                self.filters_applied = true;

                // Recreate bind group with filtered texture
                if let (Some(render_pipeline), Some(sampler)) =
                    (&self.render_pipeline, &self.sampler)
                {
                    let texture_view = dst.create_view(&wgpu::TextureViewDescriptor::default());
                    let bind_group_layout = render_pipeline.get_bind_group_layout(0);
                    self.bind_group = Some(frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Image bind group (filtered)"),
                        layout: &bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&texture_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(sampler),
                            },
                        ],
                    }));
                }
            }
        }

        // Render to screen
        let Some(render_pipeline) = &self.render_pipeline else {
            return;
        };
        let Some(bind_group) = &self.bind_group else {
            return;
        };

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Image render encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Image render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(render_pipeline);
            render_pass.set_bind_group(0, bind_group, &[]);
            render_pass.draw(0..6, 0..1); // 2 triangles = 6 vertices
        }

        frame.queue.submit([encoder.finish()]);
    }
}

/// Convenience constructor for building an Image view inline.
#[must_use]
pub fn image(pixels: Vec<u8>, width: u32, height: u32) -> Image {
    Image::new(pixels, width, height)
}
