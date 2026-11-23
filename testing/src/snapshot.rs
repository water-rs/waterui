use std::fs;
use std::io;
use std::path::Path;
#[cfg(feature = "gpu")]
use std::rc::Rc;

#[cfg(feature = "gpu")]
use hydrolysis::theme::WidgetTheme;
#[cfg(feature = "gpu")]
use hydrolysis::{
    DEFAULT_HDR_CAPTURE_HEADROOM, FontFamilyResolution, HydrolysisRenderer, OffscreenGpuContext,
    OffscreenWindow, OutputColor, Style, readback_texture_rgba8, readback_texture_rgba16f,
};
use image::{ImageError, ImageResult};
#[cfg(feature = "gpu")]
use kurbo::{Affine, Rect};
#[cfg(feature = "gpu")]
use waterui::graphics::WorkingColor;
#[cfg(feature = "gpu")]
use waterui::text::FontCollection;
#[cfg(feature = "gpu")]
use waterui_core::{AnyView, Environment, View};

#[cfg(feature = "gpu")]
use crate::artifacts::{CapturedSnapshot, TestArtifacts};
use crate::error::VisualDiffError;
#[cfg(feature = "flip")]
use crate::flip::{FlipConfig, FlipResult, compare_flip_rgba8};
use crate::image::{
    BlankFrameAnalysis, RgbaImage, analyze_blank_rgba8, encode_png_display_p3_pq_rgba16,
    encode_png_display_p3_rgba8, validate_rgba8_len,
};

/// RGBA8 frame captured from a headless hydrolysis render pass.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Snapshot {
    /// Frame width in pixels.
    width: u32,
    /// Frame height in pixels.
    height: u32,
    /// Pixel data in RGBA8 row-major order.
    rgba8: Vec<u8>,
}

impl Snapshot {
    /// Creates a Snapshot from its parts.
    #[must_use]
    pub const fn new(width: u32, height: u32, rgba8: Vec<u8>) -> Self {
        Self {
            width,
            height,
            rgba8,
        }
    }

    /// Frame width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Frame height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Pixel data in RGBA8 row-major order.
    #[must_use]
    pub const fn rgba8(&self) -> &[u8] {
        self.rgba8.as_slice()
    }

    /// Pixel data in RGBA8 row-major order, for editing in place.
    pub const fn rgba8_mut(&mut self) -> &mut [u8] {
        self.rgba8.as_mut_slice()
    }

    /// Consumes the snapshot and returns `(width, height, rgba8)`.
    #[must_use]
    pub fn into_parts(self) -> (u32, u32, Vec<u8>) {
        (self.width, self.height, self.rgba8)
    }
}

impl Snapshot {
    /// Loads a PNG file into a [`Snapshot`] using pure-Rust decoding.
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::Io`] or [`VisualDiffError::Image`] if the
    /// file cannot be read or decoded as a PNG image.
    pub fn load_png(path: impl AsRef<Path>) -> Result<Self, VisualDiffError> {
        RgbaImage::load_png(path).map(Self::from)
    }

    /// Saves the snapshot to a PNG file tagged with the Display P3 `cICP`
    /// chunk (`(12, 13, 0, 1)`).
    ///
    /// # Errors
    ///
    /// Returns image I/O or encoding errors from the underlying PNG writer.
    ///
    /// # Panics
    ///
    /// Panics if the stored RGBA buffer length does not match the snapshot dimensions.
    pub fn save_png(&self, path: impl AsRef<Path>) -> ImageResult<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(ImageError::IoError)?;
        }
        assert_eq!(
            self.rgba8.len(),
            self.width as usize * self.height as usize * 4,
            "Snapshot::save_png: rgba buffer shape must match dimensions"
        );
        let encoded =
            encode_png_display_p3_rgba8(self.width, self.height, &self.rgba8).map_err(|error| {
                ImageError::IoError(io::Error::other(format!(
                    "failed to encode Display P3 PNG: {error}"
                )))
            })?;
        fs::write(path, encoded).map_err(ImageError::IoError)
    }

    /// Analyzes the snapshot to detect blank (empty, fully transparent, or
    /// uniform single-color) captures.
    #[must_use]
    pub fn analyze_blank(&self) -> BlankFrameAnalysis {
        analyze_blank_rgba8(self.width, self.height, &self.rgba8)
    }

    /// Compares this reference snapshot against `candidate` using NVIDIA
    /// LDR-FLIP with [`FlipConfig::default`].
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::InvalidImageBuffer`] if either snapshot's
    /// buffer length does not match its dimensions,
    /// [`VisualDiffError::DimensionMismatch`] when the snapshots differ in
    /// size, or [`VisualDiffError::EmptyImage`] when either dimension is zero.
    #[cfg(feature = "flip")]
    pub fn compare_flip(&self, candidate: &Self) -> Result<FlipResult, VisualDiffError> {
        self.compare_flip_with(candidate, FlipConfig::default())
    }

    /// Compares this reference snapshot against `candidate` using NVIDIA
    /// LDR-FLIP with an explicit [`FlipConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::InvalidConfig`] for invalid parameters,
    /// [`VisualDiffError::InvalidImageBuffer`] if either snapshot's buffer
    /// length does not match its dimensions,
    /// [`VisualDiffError::DimensionMismatch`] when the snapshots differ in
    /// size, or [`VisualDiffError::EmptyImage`] when either dimension is zero.
    #[cfg(feature = "flip")]
    pub fn compare_flip_with(
        &self,
        candidate: &Self,
        config: FlipConfig,
    ) -> Result<FlipResult, VisualDiffError> {
        compare_flip_rgba8(
            self.width,
            self.height,
            &self.rgba8,
            candidate.width,
            candidate.height,
            &candidate.rgba8,
            config,
        )
    }
}

/// Extended linear Display P3 `f32` RGBA frame captured from an HDR offscreen
/// render pass (`Rgba16Float` / `OutputColor::LinearDisplayP3`).
///
/// RGB channels hold unclamped linear light in Display P3 primaries where `1.0`
/// corresponds to SDR reference white (`203` nits under ITU-R BT.2408) and
/// values above `1.0` preserve HDR / EDR highlights.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct HdrSnapshot {
    width: u32,
    height: u32,
    rgba_f32: Vec<f32>,
    headroom: f32,
}

impl HdrSnapshot {
    /// Creates an [`HdrSnapshot`] from row-major linear Display P3 `f32` RGBA
    /// pixels (`width * height * 4` floats).
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::InvalidImageBuffer`] when `rgba_f32.len()`
    /// does not equal `width * height * 4`.
    pub fn new(width: u32, height: u32, rgba_f32: Vec<f32>) -> Result<Self, VisualDiffError> {
        Self::with_headroom(width, height, rgba_f32, 10_000.0 / 203.0)
    }

    /// Creates an [`HdrSnapshot`] recording the capture target's display
    /// headroom ceiling (`>= 1.0`).
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::InvalidImageBuffer`] when `rgba_f32.len()`
    /// does not equal `width * height * 4`.
    pub fn with_headroom(
        width: u32,
        height: u32,
        rgba_f32: Vec<f32>,
        headroom: f32,
    ) -> Result<Self, VisualDiffError> {
        validate_rgba8_len(width, height, rgba_f32.len())?;
        Ok(Self {
            width,
            height,
            rgba_f32,
            headroom: if headroom.is_finite() && headroom > 1.0 {
                headroom
            } else {
                1.0
            },
        })
    }

    /// Frame width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Frame height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Row-major extended linear Display P3 `f32` RGBA channels
    /// (`width * height * 4` floats).
    #[must_use]
    pub const fn rgba_f32(&self) -> &[f32] {
        self.rgba_f32.as_slice()
    }

    /// Configured capture target headroom ceiling (`>= 1.0`).
    #[must_use]
    pub const fn headroom(&self) -> f32 {
        self.headroom
    }

    /// Consumes the snapshot and returns `(width, height, rgba_f32)`.
    #[must_use]
    pub fn into_parts(self) -> (u32, u32, Vec<f32>) {
        (self.width, self.height, self.rgba_f32)
    }

    /// Returns the `[R, G, B, A]` linear Display P3 pixel at `(x, y)`.
    ///
    /// # Panics
    ///
    /// Panics if `x >= width` or `y >= height`.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> [f32; 4] {
        assert!(
            x < self.width && y < self.height,
            "pixel ({x}, {y}) out of bounds for {}x{}",
            self.width,
            self.height
        );
        let offset = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        [
            self.rgba_f32[offset],
            self.rgba_f32[offset + 1],
            self.rgba_f32[offset + 2],
            self.rgba_f32[offset + 3],
        ]
    }

    /// Maximum finite RGB channel value across the entire frame (`0.0` if
    /// empty).
    #[must_use]
    pub fn max_channel(&self) -> f32 {
        self.rgba_f32
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|px| [px[0], px[1], px[2]])
            .filter(|v| v.is_finite())
            .fold(0.0_f32, f32::max)
    }

    /// Peak EDR headroom observed in this frame relative to SDR white
    /// (`>= 1.0`).
    #[must_use]
    pub fn max_edr_headroom(&self) -> f32 {
        self.max_channel().max(1.0)
    }

    /// Encodes this HDR snapshot as a 16-bit big-endian RGBA PNG tagged with
    /// Display P3 SMPTE ST 2084 PQ `cICP` `(12, 16, 0, 1)` at ITU-R BT.2408
    /// `203`-nit reference white.
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::InvalidImageBuffer`] or
    /// [`VisualDiffError::EncodePng`] if PNG encoding fails.
    pub fn to_hdr_png(&self) -> Result<Vec<u8>, VisualDiffError> {
        encode_png_display_p3_pq_rgba16(self.width, self.height, &self.rgba_f32)
    }

    /// Writes this HDR snapshot to `path` as a 16-bit Display P3 PQ PNG
    /// (`cICP (12, 16, 0, 1)`), creating parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns [`VisualDiffError::Io`] or [`VisualDiffError::EncodePng`] if
    /// writing or PNG encoding fails.
    pub fn save_hdr_png(&self, path: impl AsRef<Path>) -> Result<(), VisualDiffError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| VisualDiffError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let encoded = self.to_hdr_png()?;
        fs::write(path, encoded).map_err(|source| VisualDiffError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Converts this extended linear Display P3 snapshot into an 8-bit SDR
    /// Display P3 [`Snapshot`] by clamping/tone-mapping highlights to SDR white
    /// and applying the sRGB transfer function (Display P3's OETF).
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "sRGB OETF output is clamped to [0.0, 1.0] before scaling to 0..=255"
    )]
    pub fn to_sdr_snapshot(&self) -> Snapshot {
        let mut rgba8 = Vec::with_capacity(self.rgba_f32.len());
        for px in self.rgba_f32.as_chunks::<4>().0 {
            let max_rgb = px[0].max(px[1]).max(px[2]);
            let scale = if max_rgb > 1.0 { 1.0 / max_rgb } else { 1.0 };
            for ch in [px[0] * scale, px[1] * scale, px[2] * scale] {
                let encoded = linear_to_srgb(ch.clamp(0.0, 1.0));
                rgba8.push((encoded * 255.0).round() as u8);
            }
            let alpha = if px[3].is_finite() {
                px[3].clamp(0.0, 1.0)
            } else {
                0.0
            };
            rgba8.push((alpha * 255.0).round() as u8);
        }
        Snapshot::new(self.width, self.height, rgba8)
    }
}

/// IEC 61966-2-1 sRGB OETF (also used by Display P3).
fn linear_to_srgb(linear: f32) -> f32 {
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055_f32.mul_add(linear.powf(1.0 / 2.4), -0.055)
    }
}

/// Headless host that renders `WaterUI` views into an offscreen GPU texture.
///
/// The host renders with a style — the same `hydrolysis::Style` a rendered
/// mount takes — because a frame is a product of the view tree *and* the
/// style package's widget theme; a style-free render exists only on the
/// semantic pipeline, which produces no pixels. It renders on the GPU
/// painter; an [`OffscreenApp`](crate::OffscreenApp) renders on whichever
/// painter its runtime selected.
#[cfg(feature = "gpu")]
pub struct TestHost {
    env: Environment,
    /// Requested once and shared by every render this host performs. A wgpu
    /// device is expensive to request and expensive to hold on a runner whose
    /// only adapter is a software rasterizer; a host that renders ten views
    /// should ask for one device, not ten.
    gpu: OffscreenGpuContext,
    width: u32,
    height: u32,
    theme: Rc<dyn WidgetTheme>,
    /// Built once from the host's environment — system fonts plus the
    /// package's declared fonts — and shared by every render.
    fonts: FontCollection,
}

#[cfg(feature = "gpu")]
impl core::fmt::Debug for TestHost {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TestHost")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "gpu")]
impl TestHost {
    /// Creates a test host with a fixed render size, installing the style's
    /// tokens into `env` the way a rendered runtime does.
    ///
    /// # Panics
    ///
    /// Panics with the cause when the host has no GPU Hydrolysis can render
    /// on.
    #[must_use]
    pub fn new(env: Environment, width: u32, height: u32, style: impl Style) -> Self {
        let mut env = env;
        waterui::install_application_resources(&mut env);
        // A host that staged its declared fonts itself installs a
        // `DeclaredFonts`; a test binary under cargo resolves them here.
        crate::declared_fonts::install_declared_fonts(&mut env);
        hydrolysis::theme::install_default_tokens(&mut env);
        style.install_tokens(&mut env);
        let fonts = hydrolysis::native_collection(&env);
        fonts.clone().install(&mut env);
        Self {
            env,
            gpu: OffscreenGpuContext::new_blocking()
                .unwrap_or_else(|error| snapshot_failed("requesting the GPU", error)),
            width,
            height,
            theme: Rc::new(style),
            fonts,
        }
    }

    /// Renders a view and returns the captured Display P3 RGBA8 snapshot.
    ///
    /// # Panics
    ///
    /// Panics with the cause if the frame fails to render or its readback
    /// fails.
    pub fn render<V: View>(&self, view: V) -> Snapshot {
        let mut platform = OffscreenWindow::on_context(
            self.gpu.clone(),
            self.width.max(1),
            self.height.max(1),
            wgpu::TextureFormat::Rgba8Unorm,
        );
        platform.surface().set_output_color(OutputColor::DisplayP3);
        let mut renderer = HydrolysisRenderer::with_fonts(
            Rc::clone(&self.theme),
            &self.fonts,
            FontFamilyResolution::Strict,
        );
        let bounds = kurbo::Rect::new(
            0.0,
            0.0,
            f64::from(self.width.max(1)),
            f64::from(self.height.max(1)),
        );

        let surface = platform.surface();
        renderer.reset_scene();
        renderer.begin_rebuild_frame();
        renderer.capture_window_tree(
            AnyView::new(view),
            &self.env,
            bounds,
            kurbo::Affine::IDENTITY,
            kurbo::Affine::IDENTITY,
        );
        renderer.finish_rebuild_frame();

        let frame = surface
            .acquire()
            .unwrap_or_else(|error| snapshot_failed("acquiring the frame", error));
        renderer
            .render_scene_to_texture(surface, frame.texture(), WorkingColor::TRANSPARENT)
            .unwrap_or_else(|error| snapshot_failed("rendering the frame", error));
        let rgba8 = readback_texture_rgba8(
            &*surface,
            frame.texture(),
            self.width.max(1),
            self.height.max(1),
        )
        .unwrap_or_else(|error| snapshot_failed("reading back the frame", error));
        surface.present(frame);
        drop(renderer);
        drop(platform);
        // Both owners of this render's GPU resources are gone; let the device
        // release them before the next render allocates its own.
        self.gpu.reclaim();

        Snapshot {
            width: self.width.max(1),
            height: self.height.max(1),
            rgba8,
        }
    }

    /// Renders a view onto an `Rgba16Float` (`OutputColor::LinearDisplayP3`)
    /// offscreen surface with [`DEFAULT_HDR_CAPTURE_HEADROOM`] and returns the
    /// unclamped extended linear Display P3 [`HdrSnapshot`].
    ///
    /// # Panics
    ///
    /// Panics with the cause if the frame fails to render or its readback
    /// fails.
    pub fn render_hdr<V: View>(&self, view: V) -> HdrSnapshot {
        self.render_hdr_with_headroom(view, DEFAULT_HDR_CAPTURE_HEADROOM)
    }

    /// Renders a view onto an `Rgba16Float` (`OutputColor::LinearDisplayP3`)
    /// offscreen surface at an explicit display `headroom` ceiling (`>= 1.0`).
    ///
    /// # Panics
    ///
    /// Panics with the cause if the frame fails to render or its readback
    /// fails.
    pub fn render_hdr_with_headroom<V: View>(&self, view: V, headroom: f32) -> HdrSnapshot {
        let width = self.width.max(1);
        let height = self.height.max(1);
        let mut platform = OffscreenWindow::on_context(
            self.gpu.clone(),
            width,
            height,
            wgpu::TextureFormat::Rgba16Float,
        );
        platform.surface().set_headroom(headroom);
        platform
            .surface()
            .set_output_color(OutputColor::LinearDisplayP3);
        let effective_headroom = platform.surface().headroom();
        let mut renderer = HydrolysisRenderer::with_fonts(
            Rc::clone(&self.theme),
            &self.fonts,
            FontFamilyResolution::Strict,
        );
        let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));

        let surface = platform.surface();
        renderer.reset_scene();
        renderer.begin_rebuild_frame();
        renderer.capture_window_tree(
            AnyView::new(view),
            &self.env,
            bounds,
            Affine::IDENTITY,
            Affine::IDENTITY,
        );
        renderer.finish_rebuild_frame();

        let frame = surface
            .acquire()
            .unwrap_or_else(|error| snapshot_failed("acquiring the HDR frame", error));
        renderer
            .render_scene_to_texture(surface, frame.texture(), WorkingColor::TRANSPARENT)
            .unwrap_or_else(|error| snapshot_failed("rendering the HDR frame", error));
        let rgba_f32 = readback_texture_rgba16f(&*surface, frame.texture(), width, height)
            .unwrap_or_else(|error| snapshot_failed("reading back the HDR frame", error));
        surface.present(frame);
        drop(renderer);
        drop(platform);
        self.gpu.reclaim();

        HdrSnapshot::with_headroom(width, height, rgba_f32, effective_headroom)
            .expect("readback_texture_rgba16f matches width * height * 4 floats")
    }

    /// Renders a view, stores the PNG in the canonical artifact layout, and returns both.
    pub fn capture_snapshot_with<V: View>(
        &self,
        view: V,
        artifacts: &TestArtifacts,
        case: impl AsRef<str>,
        stage: impl AsRef<str>,
    ) -> CapturedSnapshot {
        artifacts.capture_snapshot(case, stage, self.render(view))
    }
    /// Creates a canonical artifact helper rooted at the provided suite.
    #[must_use]
    pub fn artifacts(&self, suite: impl AsRef<str>) -> TestArtifacts {
        TestArtifacts::new(suite.as_ref())
    }

    /// Renders a view and stores the resulting snapshot in `WaterUI`'s canonical artifact layout.
    pub fn capture_snapshot<V: View>(
        &self,
        view: V,
        suite: impl AsRef<str>,
        case: impl AsRef<str>,
        stage: impl AsRef<str>,
    ) -> CapturedSnapshot {
        let artifacts = self.artifacts(suite);
        artifacts.capture_snapshot(case, stage, self.render(view))
    }
}

/// Fails the snapshot with `error` and its whole source chain.
#[cfg(feature = "gpu")]
fn snapshot_failed(stage: &str, error: impl core::error::Error + Send + Sync + 'static) -> ! {
    panic!(
        "waterui-testing snapshot failed {stage}: {:#}",
        waterui_core::Error::from(error)
    )
}
