#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "intentional lossy numeric cast in rendering/layout code"
)]
//! GPU-accelerated Image view using wgpu.
//!
//! This module provides [`Image`], a View that displays images on the GPU.
//! Images are stored as GPU textures and rendered directly.
//!
//! # Example
//!
//! ```ignore
//! use waterui_image::Image;
//!
//! // Create an image from RGBA pixel data
//! Image::new(rgba_pixels, 800, 600)
//! ```

use alloc::borrow::{Cow, ToOwned};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::fmt;
use num_traits::ToPrimitive;

use crate::codec::{self, DecodedRgba};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};
use waterui_core::{Binding, Environment, SignalExt, View};
use waterui_graphics::{
    GpuContext, GpuFrame, GpuRuntime, GpuSurface, GpuView, OffscreenRenderConfig,
    OffscreenRenderError, OffscreenRenderOutput, OffscreenRenderOutputHdr,
};
use waterui_layout::frame::Frame;

pub use crate::codec::DecodePath;

/// A GPU-accelerated image view.
///
/// `Image` stores pixel data that will be uploaded to a GPU texture when the
/// view is set up. The pixel data is consumed during setup and not retained
/// in memory.
///
/// # Memory Model
///
/// - **Before setup**: Holds pending pixel data (`Vec<u8>`)
/// - **After setup**: Holds only GPU texture (no CPU pixel data)
///
/// This design ensures efficient memory usage by not duplicating data between
/// CPU and GPU memory.
///
/// # Example
///
/// ```ignore
/// use waterui_image::Image;
///
/// // RGBA pixel data (4 bytes per pixel)
/// let pixels: Vec<u8> = vec![255, 0, 0, 255]; // 1x1 red pixel
///
/// let image = Image::new(pixels, 1, 1);
/// ```
#[derive(Debug)]
pub struct Image {
    renderer: ImageRenderer,
    width: u32,
    height: u32,
    /// When `true`, the image stretches to fill its proposed bounds via the
    /// configured [`Interpolation`]; otherwise it locks to its native pixel
    /// size like a `SwiftUI` `Image` (the default).
    resizable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourcePixelFormat {
    Rgba8UnormSrgb,
    Rgba16Float,
}

/// How the image should be filtered when its pixel grid does not align
/// with the destination pixel grid (which it almost never does on modern
/// fractional-DPR displays).
///
/// `Linear` is the conventional default for photographs and decoded
/// assets, while `Nearest` preserves sharp pixel edges and is the right
/// choice for icons, pixel art, and rasterized barcodes.
///
/// `#[non_exhaustive]` so future modes (e.g. cubic / Lanczos) can be added
/// without breaking exhaustive `match` statements downstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Interpolation {
    /// Bilinear / trilinear sampling. Default for photo-like content.
    #[default]
    Linear,
    /// Nearest-neighbor sampling. Use for pixel art, icons, barcodes.
    Nearest,
}

impl Interpolation {
    const fn to_wgpu_filter(self) -> wgpu::FilterMode {
        match self {
            Self::Linear => wgpu::FilterMode::Linear,
            Self::Nearest => wgpu::FilterMode::Nearest,
        }
    }

    const fn to_wgpu_mipmap_filter(self) -> wgpu::MipmapFilterMode {
        match self {
            Self::Linear => wgpu::MipmapFilterMode::Linear,
            Self::Nearest => wgpu::MipmapFilterMode::Nearest,
        }
    }
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
            resizable: false,
            renderer: ImageRenderer::new(
                pixels,
                width,
                height,
                SourcePixelFormat::Rgba8UnormSrgb,
                false,
                false,
            ),
            width,
            height,
        }
    }

    /// Creates a new Image from RGBA16F pixel data.
    ///
    /// The pixel data must be in RGBA16F format (8 bytes per pixel, little-endian half-float
    /// components) and have exactly `width * height * 8` bytes.
    #[must_use]
    pub fn new_rgba16f(pixels: Vec<u8>, width: u32, height: u32) -> Self {
        Self::new_rgba16f_with_metadata(pixels, width, height, true, false)
    }

    #[must_use]
    fn new_rgba16f_with_metadata(
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        source_is_hdr: bool,
        source_is_wide_gamut: bool,
    ) -> Self {
        assert_eq!(
            pixels.len(),
            (width * height * 8) as usize,
            "Pixel data length must be width * height * 8 for RGBA16F"
        );
        Self {
            resizable: false,
            renderer: ImageRenderer::new(
                pixels,
                width,
                height,
                SourcePixelFormat::Rgba16Float,
                source_is_hdr,
                source_is_wide_gamut,
            ),
            width,
            height,
        }
    }

    /// Sets the texture sampling mode for this image.
    ///
    /// Defaults to [`Interpolation::Linear`]. Switch to
    /// [`Interpolation::Nearest`] for content that should keep crisp pixel
    /// edges across non-integer scale factors (icons, pixel art, rasterized
    /// barcodes).
    #[must_use]
    pub const fn interpolation(mut self, mode: Interpolation) -> Self {
        self.renderer.interpolation = mode;
        self
    }

    /// Allows this image to stretch to its proposed bounds instead of
    /// locking to its native pixel size.
    ///
    /// Mirrors `SwiftUI`'s `Image.resizable()`. The default behaviour
    /// frames the image to its source `width × height` so a 64-pixel
    /// asset stays 64 pixels tall regardless of the parent's proposal;
    /// once `.resizable()` is applied the image fills whatever the
    /// parent gives it and scales via the configured [`Interpolation`].
    #[must_use]
    pub const fn resizable(mut self) -> Self {
        self.resizable = true;
        self
    }

    /// Get the image dimensions (width, height).
    #[must_use]
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Get the image width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Get the image height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Decode encoded image bytes and construct a GPU-backed `Image`.
    ///
    /// # Errors
    ///
    /// Returns an error when the encoded image cannot be decoded into GPU-uploadable pixels.
    pub fn from_encoded(data: &[u8]) -> Result<Self, String> {
        codec::decode_to_rgba8(data).map(Self::from_decoded)
    }

    /// Decode encoded image bytes and report which decode path was selected.
    ///
    /// # Errors
    ///
    /// Returns an error when the encoded image cannot be decoded into GPU-uploadable pixels.
    pub fn from_encoded_with_path(data: &[u8]) -> Result<(Self, DecodePath), String> {
        codec::decode_to_rgba8_with_path(data)
            .map(|(decoded, path)| (Self::from_decoded(decoded), path))
    }

    /// Build an incremental decoder that accepts binary image stream chunks.
    #[must_use]
    pub fn stream_decoder(content_type: Option<&str>) -> ImageStreamDecoder {
        ImageStreamDecoder::new(content_type)
    }

    /// Renders this image into an offscreen RGBA8 target.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying GPU offscreen render fails.
    #[expect(
        clippy::future_not_send,
        reason = "image rendering awaits the UI-local offscreen GpuView environment"
    )]
    pub async fn render_offscreen(
        self,
        runtime: &GpuRuntime,
        config: OffscreenRenderConfig,
        env: &mut waterui_core::Environment,
    ) -> Result<OffscreenRenderOutput, OffscreenRenderError> {
        GpuSurface::new(self.renderer)
            .render_offscreen(runtime, config, env)
            .await
    }

    /// Renders this image into an HDR offscreen target and reads back `RGBA16F`.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying GPU offscreen render fails.
    #[expect(
        clippy::future_not_send,
        reason = "image rendering awaits the UI-local offscreen GpuView environment"
    )]
    pub async fn render_offscreen_hdr(
        self,
        runtime: &GpuRuntime,
        config: OffscreenRenderConfig,
        env: &mut waterui_core::Environment,
    ) -> Result<OffscreenRenderOutputHdr, OffscreenRenderError> {
        GpuSurface::new(self.renderer)
            .render_offscreen_hdr(runtime, config, env)
            .await
    }

    fn from_decoded(decoded: DecodedRgba) -> Self {
        match decoded.pixel_format {
            waterkit_codec::DecodedPixelFormat::Rgba8UnormSrgb => {
                Self::new(decoded.pixels, decoded.width, decoded.height)
            }
            waterkit_codec::DecodedPixelFormat::Rgba16Float => Self::new_rgba16f_with_metadata(
                decoded.pixels,
                decoded.width,
                decoded.height,
                decoded.hdr,
                decoded.wide_gamut,
            ),
            other => {
                panic!("Image::from_decoded: unsupported decoded pixel format: {other:?}");
            }
        }
    }
}

impl View for Image {
    fn body(self, _env: &Environment) -> impl View {
        let width = u32_to_f32(self.width);
        let height = u32_to_f32(self.height);
        let resizable = self.resizable;
        let surface = GpuSurface::new(self.renderer);
        let frame = Frame::new(surface);
        if resizable {
            frame
        } else {
            frame.width(width).height(height)
        }
    }
}

struct ImageFrame {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    source_pixel_format: SourcePixelFormat,
    source_is_hdr: bool,
    source_is_wide_gamut: bool,
    interpolation: Interpolation,
}

impl fmt::Debug for ImageFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImageFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("source_pixel_format", &self.source_pixel_format)
            .field("source_is_hdr", &self.source_is_hdr)
            .field("source_is_wide_gamut", &self.source_is_wide_gamut)
            .field("interpolation", &self.interpolation)
            .finish_non_exhaustive()
    }
}

impl Image {
    fn into_frame(mut self) -> ImageFrame {
        let pixels = self
            .renderer
            .pending_pixels
            .take()
            .expect("Image can only enter a reactive image before GPU setup");
        ImageFrame {
            pixels,
            width: self.width,
            height: self.height,
            source_pixel_format: self.renderer.source_pixel_format,
            source_is_hdr: self.renderer.source_is_hdr,
            source_is_wide_gamut: self.renderer.source_is_wide_gamut,
            interpolation: self.renderer.interpolation,
        }
    }
}

#[derive(Debug, Default)]
struct PendingImageUpdate {
    dirty: bool,
    frame: Option<ImageFrame>,
}

#[derive(Debug)]
struct ReactiveImageState {
    pending: RefCell<PendingImageUpdate>,
    dimensions: Binding<Option<(u32, u32)>>,
    redraw: RefCell<Option<waterui_graphics::RedrawHandle>>,
}

impl ReactiveImageState {
    fn publish(&self, frame: Option<ImageFrame>) {
        self.dimensions
            .set(frame.as_ref().map(|frame| (frame.width, frame.height)));
        *self.pending.borrow_mut() = PendingImageUpdate { dirty: true, frame };
        if let Some(redraw) = self.redraw.borrow().as_ref() {
            redraw.request_redraw();
        }
    }

    fn take_pending(&self) -> PendingImageUpdate {
        core::mem::take(&mut *self.pending.borrow_mut())
    }
}

/// A handle that publishes decoded frames into one persistent [`ReactiveImage`].
#[derive(Clone, Debug)]
pub struct ReactiveImageHandle {
    state: Rc<ReactiveImageState>,
}

impl ReactiveImageHandle {
    /// Replaces the displayed GPU texture without replacing the image view.
    pub fn set(&self, image: Image) {
        self.state.publish(Some(image.into_frame()));
    }

    /// Removes the displayed frame without replacing the image view.
    pub fn clear(&self) {
        self.state.publish(None);
    }
}

/// An image view whose decoded frame can change without rebuilding its subtree.
#[derive(Debug)]
pub struct ReactiveImage {
    state: Rc<ReactiveImageState>,
    resizable: bool,
}

impl ReactiveImage {
    /// Allows this image to stretch to its proposed bounds.
    #[must_use]
    pub const fn resizable(mut self) -> Self {
        self.resizable = true;
        self
    }
}

impl View for ReactiveImage {
    fn body(self, _env: &Environment) -> impl View {
        let width = self
            .state
            .dimensions
            .map(|dimensions| dimensions.map_or(0.0, |(width, _)| u32_to_f32(width)))
            .computed();
        let height = self
            .state
            .dimensions
            .map(|dimensions| dimensions.map_or(0.0, |(_, height)| u32_to_f32(height)))
            .computed();
        let surface = GpuSurface::new(ReactiveImageRenderer {
            image: ImageRenderer::empty(),
            state: Rc::clone(&self.state),
        });
        let frame = Frame::new(surface);
        if self.resizable {
            frame
        } else {
            frame.width(width).height(height)
        }
    }
}

/// Creates one persistent image view and its precise frame-update handle.
#[must_use]
pub fn reactive_image() -> (ReactiveImageHandle, ReactiveImage) {
    let state = Rc::new(ReactiveImageState {
        pending: RefCell::new(PendingImageUpdate::default()),
        dimensions: Binding::container(None),
        redraw: RefCell::new(None),
    });
    (
        ReactiveImageHandle {
            state: Rc::clone(&state),
        },
        ReactiveImage {
            state,
            resizable: false,
        },
    )
}

/// Internal GPU renderer for images.
struct ImageRenderer {
    /// Pending pixel data (consumed during setup)
    pending_pixels: Option<Vec<u8>>,
    /// Source pixel format for the pending data
    source_pixel_format: SourcePixelFormat,
    /// Whether source pixels represent HDR content.
    source_is_hdr: bool,
    /// Whether source pixels preserve wide-gamut content.
    source_is_wide_gamut: bool,
    /// Image width
    width: u32,
    /// Image height
    height: u32,
    /// Sampler filter mode chosen by the user.
    interpolation: Interpolation,
    /// GPU texture (created during setup)
    texture: Option<wgpu::Texture>,
    /// Render pipeline for displaying to screen
    render_pipeline: Option<wgpu::RenderPipeline>,
    /// Bind group for rendering
    bind_group: Option<wgpu::BindGroup>,
    /// Sampler for texture sampling
    sampler: Option<wgpu::Sampler>,
}

impl fmt::Debug for ImageRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageRenderer")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl ImageRenderer {
    const COPY_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

    const fn new(
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        source_pixel_format: SourcePixelFormat,
        source_is_hdr: bool,
        source_is_wide_gamut: bool,
    ) -> Self {
        Self {
            pending_pixels: Some(pixels),
            source_pixel_format,
            source_is_hdr,
            source_is_wide_gamut,
            width,
            height,
            interpolation: Interpolation::Linear,
            texture: None,
            render_pipeline: None,
            bind_group: None,
            sampler: None,
        }
    }

    const fn empty() -> Self {
        Self {
            pending_pixels: None,
            source_pixel_format: SourcePixelFormat::Rgba8UnormSrgb,
            source_is_hdr: false,
            source_is_wide_gamut: false,
            width: 0,
            height: 0,
            interpolation: Interpolation::Linear,
            texture: None,
            render_pipeline: None,
            bind_group: None,
            sampler: None,
        }
    }

    fn accept_frame(&mut self, frame: ImageFrame) {
        self.pending_pixels = Some(frame.pixels);
        self.source_pixel_format = frame.source_pixel_format;
        self.source_is_hdr = frame.source_is_hdr;
        self.source_is_wide_gamut = frame.source_is_wide_gamut;
        self.width = frame.width;
        self.height = frame.height;
        self.interpolation = frame.interpolation;
    }

    fn clear_frame(&mut self) {
        self.pending_pixels = None;
        self.texture = None;
        self.render_pipeline = None;
        self.bind_group = None;
        self.sampler = None;
        self.width = 0;
        self.height = 0;
    }

    #[inline]
    const fn bytes_per_pixel(source_pixel_format: SourcePixelFormat) -> u32 {
        match source_pixel_format {
            SourcePixelFormat::Rgba8UnormSrgb => 4,
            SourcePixelFormat::Rgba16Float => 8,
        }
    }

    #[inline]
    const fn align_bytes_per_row(bytes_per_row: u32) -> u32 {
        bytes_per_row.div_ceil(Self::COPY_ALIGNMENT) * Self::COPY_ALIGNMENT
    }

    fn pad_rows_for_upload(
        pixels: &[u8],
        unpadded_bpr: u32,
        padded_bpr: u32,
        height: u32,
    ) -> Vec<u8> {
        debug_assert!(padded_bpr >= unpadded_bpr);
        debug_assert_eq!(pixels.len(), (unpadded_bpr * height) as usize);

        let mut padded = vec![0u8; (padded_bpr * height) as usize];
        for row in 0..height as usize {
            let src_start = row * unpadded_bpr as usize;
            let src_end = src_start + unpadded_bpr as usize;
            let dst_start = row * padded_bpr as usize;
            let dst_end = dst_start + unpadded_bpr as usize;
            padded[dst_start..dst_end].copy_from_slice(&pixels[src_start..src_end]);
        }
        padded
    }

    fn create_render_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        hdr_to_sdr_tonemap: bool,
        interpolation: Interpolation,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout, wgpu::Sampler) {
        // Keep alpha blending enabled for both SDR and HDR targets so transparent
        // image assets composite consistently regardless of surface format.
        let blend = Some(wgpu::BlendState::ALPHA_BLENDING);
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
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let fragment_entry_point = if hdr_to_sdr_tonemap {
            "fs_main_tonemap"
        } else {
            "fs_main"
        };
        let (vertex_shader, fragment_shader) = crate::IMAGE_RENDER_SHADER.create_render_stages(
            device,
            "vs_main",
            fragment_entry_point,
        );
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Image render pipeline"),
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
            multiview_mask: None,
            cache: None,
        });

        let filter = interpolation.to_wgpu_filter();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Image sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: filter,
            min_filter: filter,
            mipmap_filter: interpolation.to_wgpu_mipmap_filter(),
            ..Default::default()
        });

        (render_pipeline, bind_group_layout, sampler)
    }

    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) {
        let Some(pixels) = self.pending_pixels.take() else {
            return;
        };
        let texture_format = match self.source_pixel_format {
            SourcePixelFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
            SourcePixelFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        };
        let bytes_per_pixel = Self::bytes_per_pixel(self.source_pixel_format);
        let unpadded_bpr = self.width * bytes_per_pixel;
        let padded_bpr = Self::align_bytes_per_row(unpadded_bpr);
        let upload_pixels: Cow<'_, [u8]> = if padded_bpr == unpadded_bpr {
            Cow::Borrowed(pixels.as_slice())
        } else {
            Cow::Owned(Self::pad_rows_for_upload(
                &pixels,
                unpadded_bpr,
                padded_bpr,
                self.height,
            ))
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Image source texture"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format,
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
            upload_pixels.as_ref(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: Some(self.height),
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        let hdr_to_sdr_tonemap =
            should_tonemap_hdr_to_sdr(self.source_pixel_format, self.source_is_hdr, target_format);
        let (render_pipeline, bind_group_layout, sampler) = Self::create_render_pipeline(
            device,
            target_format,
            hdr_to_sdr_tonemap,
            self.interpolation,
        );
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
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

        self.texture = Some(texture);
        self.render_pipeline = Some(render_pipeline);
        self.bind_group = Some(bind_group);
        self.sampler = Some(sampler);
    }
}

impl GpuView for ImageRenderer {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let intrinsic = Size::new(u32_to_f32(self.width), u32_to_f32(self.height));
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(intrinsic.width),
            proposal.height.unwrap_or(intrinsic.height),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn setup(
        &mut self,
        ctx: &GpuContext<'_>,
        _env: &mut waterui_core::Environment,
    ) -> impl core::future::Future<Output = ()> {
        tracing::debug!(
            "[ImageRenderer] setup() called with format: {:?}, size: {}x{}, source_hdr={}, source_wide_gamut={}",
            ctx.surface_format,
            self.width,
            self.height,
            self.source_is_hdr,
            self.source_is_wide_gamut
        );
        self.prepare(ctx.device, ctx.queue, ctx.surface_format);
        core::future::ready(())
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        tracing::debug!(
            "[ImageRenderer] render() called, format: {:?}, size: {}x{}, has_pipeline: {}",
            frame.format,
            frame.width,
            frame.height,
            self.render_pipeline.is_some()
        );

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

            if let (Some(render_pipeline), Some(bind_group)) =
                (&self.render_pipeline, &self.bind_group)
            {
                render_pass.set_pipeline(render_pipeline);
                render_pass.set_bind_group(0, bind_group, &[]);
                render_pass.draw(0..6, 0..1);
            }
        }

        frame.queue.submit([encoder.finish()]);
    }
}

struct ReactiveImageRenderer {
    image: ImageRenderer,
    state: Rc<ReactiveImageState>,
}

impl GpuView for ReactiveImageRenderer {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let (width, height) = self.state.dimensions.get().unwrap_or((0, 0));
        let intrinsic = Size::new(u32_to_f32(width), u32_to_f32(height));
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(intrinsic.width),
            proposal.height.unwrap_or(intrinsic.height),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn setup(
        &mut self,
        ctx: &GpuContext<'_>,
        env: &mut waterui_core::Environment,
    ) -> impl core::future::Future<Output = ()> {
        *self.state.redraw.borrow_mut() = Some(ctx.redraw_handle.clone());
        let update = self.state.take_pending();
        if update.dirty {
            match update.frame {
                Some(frame) => self.image.accept_frame(frame),
                None => self.image.clear_frame(),
            }
        }
        self.image.setup(ctx, env)
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        let update = self.state.take_pending();
        if update.dirty {
            match update.frame {
                Some(image) => {
                    self.image.accept_frame(image);
                    self.image.prepare(frame.device, frame.queue, frame.format);
                }
                None => self.image.clear_frame(),
            }
        }
        self.image.render(frame);
    }
}

impl Drop for ReactiveImageRenderer {
    fn drop(&mut self) {
        self.state.redraw.borrow_mut().take();
    }
}

const fn should_tonemap_hdr_to_sdr(
    source_pixel_format: SourcePixelFormat,
    source_is_hdr: bool,
    target_format: wgpu::TextureFormat,
) -> bool {
    source_is_hdr
        && matches!(source_pixel_format, SourcePixelFormat::Rgba16Float)
        && !matches!(
            target_format,
            wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
        )
}

/// Convenience constructor for building an Image view inline.
#[must_use]
pub fn image(pixels: Vec<u8>, width: u32, height: u32) -> Image {
    Image::new(pixels, width, height)
}

/// Incremental encoded image decoder for streaming/progressive display.
#[derive(Debug, Clone)]
pub struct ImageStreamDecoder {
    content_type: Option<String>,
    bytes: Vec<u8>,
    attempts: usize,
    next_attempt_at: usize,
    last_fingerprint: Option<u64>,
}

impl ImageStreamDecoder {
    const FIRST_ATTEMPT_BYTES: usize = 24 * 1024;
    const ATTEMPT_STEP_BYTES: usize = 96 * 1024;
    const MAX_ATTEMPTS: usize = 10;
    const MAX_BUFFER_BYTES: usize = 8 * 1024 * 1024;

    /// Creates a new stream decoder for progressive encoded image bytes.
    #[must_use]
    pub fn new(content_type: Option<&str>) -> Self {
        Self {
            content_type: content_type.map(ToOwned::to_owned),
            bytes: Vec::new(),
            attempts: 0,
            next_attempt_at: Self::FIRST_ATTEMPT_BYTES,
            last_fingerprint: None,
        }
    }

    /// Push a stream chunk and optionally produce a progressive frame.
    #[must_use]
    pub fn push_chunk(&mut self, chunk: &[u8]) -> Option<Image> {
        if chunk.is_empty() {
            return None;
        }
        self.bytes.extend_from_slice(chunk);
        let total_len = self.bytes.len();

        if self.attempts >= Self::MAX_ATTEMPTS
            || total_len < self.next_attempt_at
            || total_len > Self::MAX_BUFFER_BYTES
            || !codec::is_progressive_candidate(self.content_type.as_deref(), &self.bytes)
        {
            return None;
        }

        self.attempts += 1;
        self.next_attempt_at = total_len.saturating_add(Self::ATTEMPT_STEP_BYTES);

        let decoded = codec::decode_progressive_frame(&self.bytes)?;

        let fingerprint = frame_fingerprint(&decoded);
        if self.last_fingerprint == Some(fingerprint) {
            return None;
        }
        self.last_fingerprint = Some(fingerprint);
        Some(Image::from_decoded(decoded))
    }

    /// Finish decoding and produce the final full-quality image.
    pub fn finish(self) -> Result<Image, String> {
        if self.bytes.is_empty() {
            return Err(String::from("image response body was empty"));
        }
        Image::from_encoded(&self.bytes)
    }
}

fn frame_fingerprint(decoded: &DecodedRgba) -> u64 {
    let len = decoded.pixels.len();
    if len == 0 {
        return 0;
    }
    let first = u64::from(decoded.pixels[0]);
    let mid = u64::from(decoded.pixels[len / 2]);
    let last = u64::from(decoded.pixels[len - 1]);
    (u64::from(decoded.width) << 32)
        ^ u64::from(decoded.height)
        ^ (u64::try_from(len).expect("image fingerprint length must fit in u64") << 8)
        ^ first
        ^ (mid << 16)
        ^ (last << 24)
}

fn u32_to_f32(value: u32) -> f32 {
    value
        .to_f32()
        .expect("image dimensions must be representable as f32")
}

#[cfg(test)]
mod tests {
    use super::{Image, SourcePixelFormat, reactive_image, should_tonemap_hdr_to_sdr};

    #[test]
    fn reactive_image_replaces_frame_without_replacing_view() {
        let (handle, _view) = reactive_image();
        handle.set(Image::new(vec![0, 0, 0, 255], 1, 1));

        assert_eq!(handle.state.dimensions.get(), Some((1, 1)));
        let update = handle.state.take_pending();
        assert!(update.dirty, "published frame must be pending");
        let frame = update.frame.expect("published update must contain a frame");
        assert_eq!((frame.width, frame.height), (1, 1));
        assert!(!handle.state.take_pending().dirty);

        handle.clear();
        assert_eq!(handle.state.dimensions.get(), None);
        let update = handle.state.take_pending();
        assert!(update.dirty);
        assert!(update.frame.is_none());
    }

    #[test]
    fn tonemap_only_for_hdr_float_source_into_sdr_target() {
        assert!(should_tonemap_hdr_to_sdr(
            SourcePixelFormat::Rgba16Float,
            true,
            wgpu::TextureFormat::Rgba8Unorm
        ));
        assert!(!should_tonemap_hdr_to_sdr(
            SourcePixelFormat::Rgba16Float,
            false,
            wgpu::TextureFormat::Rgba8Unorm
        ));
        assert!(!should_tonemap_hdr_to_sdr(
            SourcePixelFormat::Rgba16Float,
            true,
            wgpu::TextureFormat::Rgba16Float
        ));
        assert!(!should_tonemap_hdr_to_sdr(
            SourcePixelFormat::Rgba8UnormSrgb,
            true,
            wgpu::TextureFormat::Rgba8Unorm
        ));
    }
}
