//! GPU-accelerated Image view using wgpu.
//!
//! This module provides [`Image`], a View that displays images on the GPU.
//! Images are stored as GPU textures and rendered directly.
//!
//! # Example
//!
//! ```ignore
//! use waterui_media::Image;
//!
//! // Create an image from RGBA pixel data
//! Image::new(rgba_pixels, 800, 600)
//! ```

use waterui_core::{Environment, View};
use waterui_graphics::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};
use waterui_layout::frame::Frame;

/// A GPU-accelerated image view.
///
/// `Image` stores pixel data that will be uploaded to a GPU texture when the
/// view is set up. The pixel data is consumed during setup and not retained
/// in memory.
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
/// let image = Image::new(pixels, 1, 1);
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
        let width = self.renderer.width as f32;
        let height = self.renderer.height as f32;
        Frame::new(GpuSurface::new(self.renderer))
            .width(width)
            .height(height)
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
    /// Render pipeline for displaying to screen
    render_pipeline: Option<wgpu::RenderPipeline>,
    /// Bind group for rendering
    bind_group: Option<wgpu::BindGroup>,
    /// Sampler for texture sampling
    sampler: Option<wgpu::Sampler>,
}

impl core::fmt::Debug for ImageRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ImageRenderer")
            .field("width", &self.width)
            .field("height", &self.height)
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
            render_pipeline: None,
            bind_group: None,
            sampler: None,
        }
    }

    fn create_render_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        pipeline_cache: Option<&wgpu::PipelineCache>,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
        let blend = if matches!(
            format,
            wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
        ) {
            None
        } else {
            Some(wgpu::BlendState::ALPHA_BLENDING)
        };
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
                    blend,
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
            cache: pipeline_cache,
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
    fn setup(&mut self, ctx: &GpuContext) -> impl core::future::Future<Output = ()> {
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
                    | wgpu::TextureUsages::COPY_DST,
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

        // Create render pipeline
        let (render_pipeline, bind_group_layout, sampler) =
            Self::create_render_pipeline(ctx.device, ctx.surface_format, ctx.pipeline_cache);

        let texture = self.texture.as_ref().expect("Texture should be created");
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

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

        async {} // Sync renderer - immediately ready
    }

    fn render(&mut self, frame: &GpuFrame) {
        tracing::debug!(
            "[ImageRenderer] render() called, format: {:?}, size: {}x{}, has_pipeline: {}",
            frame.format,
            frame.width,
            frame.height,
            self.render_pipeline.is_some()
        );

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
