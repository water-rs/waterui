//! Device sharing and presentation of the engine's retained working-space output.

pub use crate::render::filter::EffectBox;
pub use crate::render::present::{
    DestinationPrimaries, DisplayProbe, OutputAlpha, OutputColor, OutputRequest, OutputSelection,
    Presenter, SelectionReason, TextureOutput, TransferEncoding, select_output,
    surface_output_alpha,
};
pub use crate::render::shaders::{ShaderDelivery, delivery as shader_delivery};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The device types and drawing contexts exposed to custom GPU producers.
pub mod wgpu {
    pub use ::wgpu::*;
    use std::time::Duration;

    /// Persistent device resources supplied once before rendering content.
    #[derive(Debug)]
    pub struct Context<'a> {
        /// Adapter that owns the engine's device.
        pub adapter: &'a Adapter,
        /// Engine-owned device.
        pub device: &'a Device,
        /// Engine-owned submission queue.
        pub queue: &'a Queue,
        /// Output texture format, containing premultiplied linear Display P3.
        pub format: TextureFormat,
        /// Requests a new frame after asynchronous producer work completes.
        pub redraw: super::RedrawHandle,
    }

    /// An engine-allocated output texture for one custom content frame.
    #[derive(Debug)]
    pub struct Frame<'a> {
        /// Engine-owned device.
        pub device: &'a Device,
        /// Engine-owned queue.
        pub queue: &'a Queue,
        /// Output texture in premultiplied linear Display P3.
        pub texture: &'a Texture,
        /// Output attachment view.
        pub view: &'a TextureView,
        /// Output format.
        pub format: TextureFormat,
        /// Texture width in physical pixels.
        pub width: u32,
        /// Texture height in physical pixels.
        pub height: u32,
        /// Display scale.
        pub scale: f32,
        /// Presentation time relative to this producer's first frame.
        pub elapsed: Duration,
        /// Time since the previous presentation of this content.
        pub delta: Duration,
        pub(crate) redraw: bool,
    }

    impl Frame<'_> {
        /// Keeps the engine refreshing for the next frame.
        pub const fn request_redraw(&mut self) {
            self.redraw = true;
        }
    }
}

/// A producer moved to the engine's render thread for its entire lifetime.
/// UI-thread-bound producers send owned frame data over a channel to this object.
pub trait GpuContent: cherenkov::RenderTransfer + 'static {
    /// Creates persistent resources before the first frame the producer
    /// draws. May run more than once — on a new device each time — and
    /// each run replaces every device resource it created before; a
    /// device replacement drains the producers, hands them to the new
    /// renderer, and their first drawn binding runs `setup` there.
    fn setup(&mut self, context: &wgpu::Context<'_>) -> impl Future<Output = ()>;
    /// Draws into the provided engine-owned attachment.
    fn render(&mut self, frame: &mut wgpu::Frame<'_>);
}

/// A producer boxed for `Engine::gpu_producer`.
pub struct GpuContentBox {
    pub(crate) content: Box<dyn Content>,
    pub(crate) redraw: RedrawHandle,
}

impl std::fmt::Debug for GpuContentBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuContentBox").finish_non_exhaustive()
    }
}

impl GpuContentBox {
    /// Boxes a producer. Its redraw requests wake the hosts of the
    /// surfaces that draw it, each through the wake the surface was
    /// created with.
    #[must_use]
    pub fn new(content: impl GpuContent) -> Self {
        Self {
            content: Box::new(content),
            redraw: RedrawHandle {
                dirty: Arc::new(AtomicBool::new(true)),
                wakes: Arc::new(cherenkov::SurfaceWakes::default()),
            },
        }
    }

    /// Obtains a redraw requester before the producer moves to the engine.
    #[must_use]
    pub fn redraw_handle(&self) -> RedrawHandle {
        self.redraw.clone()
    }
}

/// A thread-safe request to redraw this content on the next engine frame.
#[derive(Clone)]
pub struct RedrawHandle {
    pub(crate) dirty: Arc<AtomicBool>,
    /// The wakes of the surfaces whose frames draw the content.
    pub(crate) wakes: Arc<cherenkov::SurfaceWakes>,
}

impl std::fmt::Debug for RedrawHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedrawHandle")
            .field("dirty", &self.is_dirty())
            .finish_non_exhaustive()
    }
}

impl RedrawHandle {
    /// Marks the producer's output stale and wakes the host of each
    /// surface whose frames draw it, through the surface's own wake.
    /// Requests coalesce until a frame consumes them. Detached or removed
    /// content, and content on a surface the host has announced hidden,
    /// retains the request without waking the host; the frame that draws
    /// it again draws its latest state.
    pub fn request_redraw(&self) {
        if !self.dirty.swap(true, Ordering::AcqRel) {
            self.wakes.wake();
        }
    }

    /// Whether the producer has an unconsumed redraw request.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }
}

pub(crate) trait Content: cherenkov::RenderTransfer {
    #[cfg(not(target_arch = "wasm32"))]
    fn setup(&mut self, context: &wgpu::Context<'_>);
    #[cfg(target_arch = "wasm32")]
    fn setup<'a>(
        &'a mut self,
        context: &'a wgpu::Context<'a>,
    ) -> core::pin::Pin<Box<dyn Future<Output = ()> + 'a>>;
    fn render(&mut self, frame: &mut wgpu::Frame<'_>);
}

impl<C: GpuContent> Content for C {
    #[cfg(not(target_arch = "wasm32"))]
    fn setup(&mut self, context: &wgpu::Context<'_>) {
        pollster::block_on(GpuContent::setup(self, context));
    }

    #[cfg(target_arch = "wasm32")]
    fn setup<'a>(
        &'a mut self,
        context: &'a wgpu::Context<'a>,
    ) -> core::pin::Pin<Box<dyn Future<Output = ()> + 'a>> {
        Box::pin(GpuContent::setup(self, context))
    }

    fn render(&mut self, frame: &mut wgpu::Frame<'_>) {
        GpuContent::render(self, frame);
    }
}

/// An existing device shared with a native presentation host.
/// All four handles must belong to the same device creation chain.
///
/// On Vulkan and Metal adapters the device must be created with
/// `wgpu::Features::PASSTHROUGH_SHADERS`: the engine's fixed shaders are
/// precompiled binaries loaded through the passthrough API (issue #57), so
/// `Engine::new` fails explicitly on a device created without the feature.
#[derive(Clone, Debug)]
pub struct SharedDevice {
    /// Instance used to create the adapter and native surfaces.
    pub instance: wgpu::Instance,
    /// Adapter used to create the device.
    pub adapter: wgpu::Adapter,
    /// Device used for both engine composition and native presentation.
    /// Request `Features::PASSTHROUGH_SHADERS` on Vulkan and Metal.
    pub device: wgpu::Device,
    /// This device's submission queue.
    pub queue: wgpu::Queue,
}

/// A surface whose engine-owned linear P3 texture is handed to a native host.
///
/// The host receives a new texture only on creation and resize, then samples
/// the retained texture after `Engine::render` completes. Presentation must
/// use the same device supplied through `GpuConfig::device`. Dropping the
/// receiver stops notifications; it does not destroy the surface. A host must
/// consume resize notifications before sampling output after a resize.
#[derive(Debug)]
pub struct TextureTarget {
    pub(crate) size: (u32, u32),
    pub(crate) refresh: cherenkov::RefreshRange,
    pub(crate) textures: std::sync::mpsc::Sender<wgpu::Texture>,
}

impl TextureTarget {
    /// Sets the refresh range for animated content.
    ///
    /// # Panics
    /// When the range is empty or includes zero.
    #[must_use]
    pub fn rate(mut self, rate: cherenkov::RefreshRange) -> Self {
        assert!(
            *rate.start() > 0 && !rate.is_empty(),
            "refresh range must be positive and ordered"
        );
        self.refresh = rate;
        self
    }

    /// Creates a target and its resize notification channel.
    /// Textures contain premultiplied linear Display P3, in `Rgba16Float`.
    #[must_use]
    pub fn new(size: (u32, u32)) -> (Self, std::sync::mpsc::Receiver<wgpu::Texture>) {
        let (textures, receiver) = std::sync::mpsc::channel();
        (
            Self {
                size,
                textures,
                refresh: cherenkov::DEFAULT_REFRESH,
            },
            receiver,
        )
    }
}

/// Compiles producer WGSL with color conversion helpers.
///
/// `cherenkov_srgb`
/// accepts straight-alpha encoded sRGB; `cherenkov_premultiplied_srgb` accepts
/// encoded-domain premultiplied sRGB. Both return premultiplied linear Display P3.
/// Extended signed components are preserved.
///
/// # Panics
/// Invalid WGSL is reported through wgpu's configured error handler.
#[must_use]
pub fn shader_module(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(
            format!("{}\n{source}", include_str!("render/color.wgsl")).into(),
        ),
    })
}

/// A retained external frame the engine composites in place.
///
/// The frame references textures that live on the engine's shared device;
/// the engine samples them where the owning layer composes, with no plane
/// copy and no raster fallback. Installing a frame is
/// `engine.external_frame(frame)` followed by `tx[&layer].content(handle)`:
/// the planes stay alive until the frame is replaced, the layer's content is
/// detached or replaced, or the surface or engine is torn down.
///
/// A new frame wakes the surface through the same coalesced waker as
/// recorded content; the engine never polls the producer.
#[derive(Debug, Clone)]
pub struct ExternalFrame {
    /// The textures the fragment stage samples.
    pub planes: FramePlanes,
    /// How the planes decode into the working space.
    pub color: FrameColor,
    /// An optional GPU-side wait the sampled planes are ordered behind.
    pub wait: Option<FrameSync>,
}

/// The planes an [`ExternalFrame`] samples.
#[derive(Debug, Clone)]
pub enum FramePlanes {
    /// Two-plane 4:2:0 YUV: one luma plane and one interleaved chroma plane.
    ///
    /// NV12 is `R8Uint` + `Rg8Uint`; P010 is `R16Uint` + `Rg16Uint`. For P010
    /// the low six padding bits of every code are stripped before decode.
    /// Chroma dimensions must be `ceil(luma / 2)` on each axis.
    Yuv {
        /// The luma plane: `R8Uint` for NV12, `R16Uint` for P010.
        y: wgpu::Texture,
        /// The interleaved chroma plane: `Rg8Uint` for NV12, `Rg16Uint` for
        /// P010.
        uv: wgpu::Texture,
    },
    /// A single interleaved RGB(A) plane.
    ///
    /// `Rgba8Unorm`, `Bgra8Unorm` or `Rgba16Float`; gamma-encoded formats are
    /// rejected because the decoder applies `color.transfer` itself.
    Rgb {
        /// The pixel plane.
        plane: wgpu::Texture,
        /// How the plane's alpha channel composes.
        alpha: RgbAlpha,
    },
    /// A frame imported natively on Vulkan — its planes are bound in the
    /// engine's Vulkan external-frame operation rather than as ordinary
    /// `wgpu` textures.
    ///
    /// Import through [`vulkan::Device`]; install through
    /// [`ExternalFrame::native`].
    #[cfg(all(unix, not(target_vendor = "apple")))]
    Native(vulkan::Frame),
}

/// How a [`FramePlanes::Rgb`] plane's alpha composes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbAlpha {
    /// Every pixel is fully opaque.
    Opaque,
    /// Straight (unpremultiplied) alpha.
    Straight,
    /// Premultiplied alpha in the encoded domain.
    Premultiplied,
}

/// The `Y'CbCr` matrix of a [`FramePlanes::Yuv`] frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YuvMatrix {
    /// BT.601 (`Kr = 0.299`, `Kb = 0.114`).
    Bt601,
    /// BT.709 (`Kr = 0.2126`, `Kb = 0.0722`).
    Bt709,
    /// BT.2020 (`Kr = 0.2627`, `Kb = 0.0593`).
    Bt2020,
}

/// The code range of a [`FramePlanes::Yuv`] frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YuvRange {
    /// Studio range: luma `16..=235`, chroma `16..=240` at 8 bits.
    Video,
    /// Full code range.
    Full,
}

/// The position of chroma samples on one axis, relative to luma samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaOffset {
    /// Co-sited with the even luma sample.
    Cosited,
    /// Centred between the two luma samples the chroma covers.
    Centered,
}

/// The two-axis chroma sample location of a [`FramePlanes::Yuv`] frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromaSiting {
    /// Chroma position on the horizontal axis.
    pub x: ChromaOffset,
    /// Chroma position on the vertical axis.
    pub y: ChromaOffset,
}

impl ChromaSiting {
    /// Centered on both axes (JPEG convention).
    pub const CENTERED: Self = Self {
        x: ChromaOffset::Centered,
        y: ChromaOffset::Centered,
    };
    /// Co-sited horizontally, centered vertically (MPEG-2 convention).
    pub const LEFT: Self = Self {
        x: ChromaOffset::Cosited,
        y: ChromaOffset::Centered,
    };
    /// Co-sited on both axes.
    pub const TOP_LEFT: Self = Self {
        x: ChromaOffset::Cosited,
        y: ChromaOffset::Cosited,
    };
}

/// The additive primaries a frame's decoded signal is expressed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primaries {
    /// BT.709 / sRGB primaries.
    Bt709,
    /// Display P3 primaries.
    DisplayP3,
    /// BT.2020 primaries.
    Bt2020,
}

/// The opto-electronic transfer of a frame's encoded signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    /// Already linear light.
    Linear,
    /// The sRGB piecewise curve.
    Srgb,
    /// BT.709-encoded video. Decodes through the BT.1886 reference EOTF —
    /// a pure 2.4 power with black level 0, display-referred with reference
    /// white at 1.0 — the curve a display applies to `ITU_R_709_2` content.
    Bt709,
    /// SMPTE ST 2084 perceptual quantizer; decodes to absolute nits.
    Pq,
    /// The BT.2100 hybrid log-gamma scene transfer; the signal's own OOTF
    /// applies at decode.
    Hlg,
}

/// How a frame's planes decode into the working space.
///
/// For YUV planes the matrix, range and chroma siting decode `Y'CbCr` codes
/// into `R'G'B'`; the primaries, transfer and reference level then map that
/// signal into extended linear Display P3, the engine's working space, where
/// 1.0 is reference white. For RGB planes only the primaries, transfer and
/// reference level apply; the YUV fields are ignored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameColor {
    /// The `Y'CbCr` matrix. Ignored for RGB planes.
    pub matrix: YuvMatrix,
    /// The code range. Ignored for RGB planes.
    pub range: YuvRange,
    /// The two-axis chroma sample location. Ignored for RGB planes.
    pub chroma_siting: ChromaSiting,
    /// The primaries the decoded signal is expressed on.
    pub primaries: Primaries,
    /// The transfer the signal is encoded with.
    pub transfer: Transfer,
    /// The nits reference white occupies for this signal.
    ///
    /// Absolute transfers ([`Transfer::Pq`], [`Transfer::Hlg`]) decode to
    /// nits and are divided by this to reach the engine's white-relative
    /// working space; for relative transfers it is documentation only — the
    /// decoded signal already reaches 1.0 at reference white.
    pub reference_white: f32,
    /// The nits the HLG OOTF was produced for. Only read for
    /// [`Transfer::Hlg`].
    pub hlg_peak: f32,
}

impl FrameColor {
    /// BT.709 studio-range video with BT.709 transfer and primaries.
    pub const BT709_VIDEO: Self = Self {
        matrix: YuvMatrix::Bt709,
        range: YuvRange::Video,
        chroma_siting: ChromaSiting::LEFT,
        primaries: Primaries::Bt709,
        transfer: Transfer::Bt709,
        reference_white: 203.0,
        hlg_peak: 0.0,
    };
    /// BT.2020 studio-range video with PQ transfer on BT.2020 primaries.
    pub const BT2020_PQ: Self = Self {
        matrix: YuvMatrix::Bt2020,
        range: YuvRange::Video,
        chroma_siting: ChromaSiting::LEFT,
        primaries: Primaries::Bt2020,
        transfer: Transfer::Pq,
        reference_white: 203.0,
        hlg_peak: 0.0,
    };
    /// BT.2020 studio-range video with HLG transfer on BT.2020 primaries.
    ///
    /// `peak` is the display peak the HLG OOTF was produced for, in nits.
    #[must_use]
    pub const fn bt2020_hlg(peak: f32) -> Self {
        Self {
            matrix: YuvMatrix::Bt2020,
            range: YuvRange::Video,
            chroma_siting: ChromaSiting::LEFT,
            primaries: Primaries::Bt2020,
            transfer: Transfer::Hlg,
            reference_white: 203.0,
            hlg_peak: peak,
        }
    }
    /// sRGB on BT.709 primaries, for 8-bit RGB planes.
    pub const SRGB: Self = Self {
        matrix: YuvMatrix::Bt709,
        range: YuvRange::Full,
        chroma_siting: ChromaSiting::CENTERED,
        primaries: Primaries::Bt709,
        transfer: Transfer::Srgb,
        reference_white: 203.0,
        hlg_peak: 0.0,
    };
    /// Linear-light Display P3, for `Rgba16Float` RGB planes.
    pub const LINEAR_P3: Self = Self {
        matrix: YuvMatrix::Bt709,
        range: YuvRange::Full,
        chroma_siting: ChromaSiting::CENTERED,
        primaries: Primaries::DisplayP3,
        transfer: Transfer::Linear,
        reference_white: 203.0,
        hlg_peak: 0.0,
    };
}

/// Static HDR metadata a producer attaches to a frame.
///
/// The engine's own composition decodes a frame from its [`FrameColor`]
/// alone; this metadata travels with a frame promoted to a system
/// compositor plane, where the system's tone mapping reads it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HdrMetadata {
    /// SMPTE ST 2086 mastering display colour volume.
    pub mastering: Option<MasteringDisplay>,
    /// CTA-861.3 content light levels.
    pub content_light: Option<ContentLight>,
}

/// SMPTE ST 2086 mastering display colour volume: CIE 1931 xy
/// chromaticities of the mastering display's primaries and white point, and
/// its luminance range in nits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MasteringDisplay {
    /// The red primary's `[x, y]`.
    pub red: [f32; 2],
    /// The green primary's `[x, y]`.
    pub green: [f32; 2],
    /// The blue primary's `[x, y]`.
    pub blue: [f32; 2],
    /// The white point's `[x, y]`.
    pub white: [f32; 2],
    /// Peak luminance in nits.
    pub max_luminance: f32,
    /// Minimum luminance in nits.
    pub min_luminance: f32,
}

/// CTA-861.3 content light levels, in nits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContentLight {
    /// Maximum content light level (`MaxCLL`).
    pub max_content: f32,
    /// Maximum frame-average light level (`MaxFALL`).
    pub max_frame_average: f32,
}

/// A GPU-side wait an [`ExternalFrame`] is ordered behind.
///
/// The engine encodes the wait on the queue inside the submission that
/// samples the frame's planes; it never waits on the CPU. A producer
/// signalling later work simply installs the next frame with its own sync.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum FrameSync {
    /// A Metal shared event and the value it must reach.
    ///
    /// The producer signals `value` on its own submission; the engine's wait
    /// runs on the GPU before the frame's planes are read.
    #[cfg(target_vendor = "apple")]
    Metal {
        /// The shared event to wait on.
        event: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLSharedEvent>>,
        /// The value the event must reach.
        value: u64,
    },
}

/// Why an [`ExternalFrame`] was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidFrame {
    /// A plane's pixel format does not match its role.
    #[error(
        "a YUV frame needs R8Uint/Rg8Uint (NV12) or R16Uint/Rg16Uint (P010) planes; \
         an RGB frame needs Rgba8Unorm, Bgra8Unorm or Rgba16Float"
    )]
    PlaneFormat,
    /// The chroma plane is not `ceil(luma / 2)` on each axis, or a plane is
    /// empty.
    #[error("the chroma plane must be ceil(luma / 2) on each axis and no plane may be empty")]
    PlaneDimensions,
    /// A plane is not a single 2D mip level.
    #[error("a frame plane must be a single-layer 2D texture with one mip level")]
    PlaneGeometry,
    /// A plane was created without `TEXTURE_BINDING`.
    #[error("a frame plane needs TextureUsages::TEXTURE_BINDING")]
    PlaneUsage,
    /// `reference_white` or `hlg_peak` is not positive and finite.
    #[error("reference_white and hlg_peak must be positive and finite")]
    ColorLevel,
}

fn check_plane(texture: &wgpu::Texture) -> Result<wgpu::Extent3d, InvalidFrame> {
    let size = texture.size();
    if texture.dimension() != wgpu::TextureDimension::D2
        || size.depth_or_array_layers != 1
        || texture.mip_level_count() != 1
    {
        return Err(InvalidFrame::PlaneGeometry);
    }
    if size.width == 0 || size.height == 0 {
        return Err(InvalidFrame::PlaneDimensions);
    }
    if !texture
        .usage()
        .contains(wgpu::TextureUsages::TEXTURE_BINDING)
    {
        return Err(InvalidFrame::PlaneUsage);
    }
    Ok(size)
}

fn check_color(color: &FrameColor) -> Result<(), InvalidFrame> {
    if color.reference_white <= 0.0
        || !color.reference_white.is_finite()
        || (color.transfer == Transfer::Hlg
            && (color.hlg_peak <= 0.0 || !color.hlg_peak.is_finite()))
    {
        return Err(InvalidFrame::ColorLevel);
    }
    Ok(())
}

impl ExternalFrame {
    /// A two-plane 4:2:0 YUV frame: `y` is the luma plane and `uv` the
    /// interleaved chroma plane.
    ///
    /// # Errors
    /// [`InvalidFrame`] when a plane's format, geometry or usage does not
    /// meet the contract, or `color`'s levels are invalid.
    pub fn yuv(
        y: wgpu::Texture,
        uv: wgpu::Texture,
        color: FrameColor,
    ) -> Result<Self, InvalidFrame> {
        let y_size = check_plane(&y)?;
        let uv_size = check_plane(&uv)?;
        let yuv = matches!(
            (y.format(), uv.format()),
            (wgpu::TextureFormat::R8Uint, wgpu::TextureFormat::Rg8Uint)
                | (wgpu::TextureFormat::R16Uint, wgpu::TextureFormat::Rg16Uint)
        );
        if !yuv {
            return Err(InvalidFrame::PlaneFormat);
        }
        if uv_size.width != y_size.width.div_ceil(2) || uv_size.height != y_size.height.div_ceil(2)
        {
            return Err(InvalidFrame::PlaneDimensions);
        }
        check_color(&color)?;
        Ok(Self {
            planes: FramePlanes::Yuv { y, uv },
            color,
            wait: None,
        })
    }

    /// A single-plane RGB frame.
    ///
    /// # Errors
    /// [`InvalidFrame`] when the plane's format, geometry or usage does not
    /// meet the contract, or `color`'s levels are invalid.
    pub fn rgb(
        plane: wgpu::Texture,
        alpha: RgbAlpha,
        color: FrameColor,
    ) -> Result<Self, InvalidFrame> {
        check_plane(&plane)?;
        if !matches!(
            plane.format(),
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Rgba16Float
        ) {
            return Err(InvalidFrame::PlaneFormat);
        }
        check_color(&color)?;
        Ok(Self {
            planes: FramePlanes::Rgb { plane, alpha },
            color,
            wait: None,
        })
    }

    /// The frame's alpha contract: `Rgb` planes declare theirs; YUV
    /// planes and an opaque sampler conversion decode to full opacity.
    #[must_use]
    #[cfg_attr(
        not(all(unix, not(target_vendor = "apple"))),
        expect(
            clippy::missing_const_for_fn,
            reason = "the Native arm reads through an Arc, which cannot be const"
        )
    )]
    pub fn alpha(&self) -> RgbAlpha {
        match &self.planes {
            FramePlanes::Yuv { .. } => RgbAlpha::Opaque,
            FramePlanes::Rgb { alpha, .. } => *alpha,
            #[cfg(all(unix, not(target_vendor = "apple")))]
            FramePlanes::Native(frame) => match frame.repr() {
                vulkan::Repr::Rgb { .. } => frame.generation.alpha,
                vulkan::Repr::Planes { .. } | vulkan::Repr::ExternalFormat { .. } => {
                    RgbAlpha::Opaque
                }
            },
        }
    }

    /// A frame imported natively on the engine's Vulkan device.
    ///
    /// `frame` must come from a [`vulkan::Device`] created over the same
    /// `SharedDevice` the engine runs on; a frame imported on another
    /// `VkDevice` composes against a queue it was not acquired on and is
    /// rejected.
    ///
    /// # Errors
    /// [`InvalidFrame`] when the frame's colour levels are invalid.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    pub fn native(frame: vulkan::Frame) -> Result<Self, InvalidFrame> {
        check_color(&frame.generation.color)?;
        Ok(Self {
            color: frame.generation.color,
            planes: FramePlanes::Native(frame),
            wait: None,
        })
    }

    /// Orders the sampled planes behind a GPU-side sync.
    ///
    /// The wait runs on the GPU inside the submission that reads the planes.
    /// Only Apple targets have a sync primitive, so only they offer this.
    #[cfg(target_vendor = "apple")]
    #[must_use]
    pub fn sync(mut self, sync: FrameSync) -> Self {
        self.wait = Some(sync);
        self
    }
}

/// Native external-frame import on Vulkan (issue #166).
///
/// [`Device`](vulkan::Device) imports a producer
/// [`FrameSource`](vulkan::FrameSource) — a Linux [`DmaBuf`](vulkan::DmaBuf)
/// or an Android `AHardwareBuffer` — as a [`Frame`](vulkan::Frame) on the
/// engine's shared `VkDevice`, synchronised through a Vulkan semaphore on
/// the GPU. Install the frame on a layer through [`ExternalFrame::native`].
#[cfg(all(unix, not(target_vendor = "apple")))]
pub mod vulkan {
    #[cfg(target_os = "android")]
    pub use crate::render::external::vulkan::Ahb;
    // The producer-facing surface: import descriptors, the imported frame,
    // its capability record, the per-device context `Device::shared` and
    // `Native::new` carry, and the sync contract. The encode-side
    // machinery (`Release`, `Views`, `submit_waits`, `mark_submitted`,
    // `drain_releases`, `create_pool`, `KIND_*`) stays `pub(crate)`;
    // `Generation`/`State` and the staging pair remain public for the
    // standalone Android device-test binary, recorded in docs/api.md.
    pub use crate::render::external::vulkan::{
        Caps, Device, DmaBuf, DmaBufPlane, Frame, FrameSource, Generation, Native, NativeError,
        PendingAcquire, PendingWait, QueueFamily, ReleaseSync, Repr, Shared, State, Wait,
        cancel_staged, stage_acquire,
    };
}

/// Linux interop: presenting through exported DMA-BUFs (#1687).
///
/// [`DmabufTarget`](dmabuf::DmabufTarget) is the zero-copy present target
/// for hosts that can import Linux dma-bufs (e.g. `GdkDmabufTexture`, a
/// Wayland compositor, GStreamer): the engine renders each frame into one
/// image of a small pool of exportable Vulkan images and hands the host the
/// image's planes, an explicit DRM format modifier and a sync-file acquire
/// fence. The host returns each image with a release sync file; an image is
/// reused only after that release has signalled, and a surface with no free
/// image waits for a release rather than allocating — never a CPU wait on
/// the render thread.
#[cfg(target_os = "linux")]
pub mod dmabuf {
    use std::os::fd::OwnedFd;
    use std::sync::mpsc::{Receiver, Sender};

    pub use crate::render::external::vulkan::dmabuf::{
        DRM_FORMAT_ABGR8888, DRM_FORMAT_ARGB8888, DRM_FORMAT_XBGR8888, DRM_FORMAT_XRGB8888,
    };

    use super::{FrameColor, OutputAlpha, OutputColor, RgbAlpha};

    /// `DRM_FORMAT_ABGR16161616F` — linear half-float RGBA, for hosts
    /// that take wide-gamut/HDR pixels without an encode step.
    pub const DRM_FORMAT_ABGR16161616F: u32 = 0x4834_4241;

    /// `DRM_FORMAT_MOD_LINEAR` — an exported linear (row-major) image.
    ///
    /// `Linear` is itself an explicit DRM modifier: a `VkImage` created
    /// with `VK_IMAGE_TILING_LINEAR` plus an exported plane layout is
    /// exactly what `DRM_FORMAT_MOD_LINEAR` describes, so a host
    /// declaring it stays honest even where the driver lacks
    /// `VK_EXT_image_drm_format_modifier`. Other modifiers always
    /// require that extension.
    pub const DRM_FORMAT_MOD_LINEAR: u64 = 0;

    /// One colour plane of a presented frame: the file descriptor the
    /// plane's bytes live behind plus its byte offset and row stride.
    ///
    /// Ownership of `fd` transfers to the host; it stays valid until
    /// closed, so the host may import it at its own pace. Offsets and
    /// strides are the driver's own `vkGetImageSubresourceLayout` values.
    #[derive(Debug)]
    #[non_exhaustive]
    pub struct DmabufPlane {
        /// The dma-buf descriptor for this plane.
        pub fd: OwnedFd,
        /// Byte offset of the plane inside the buffer.
        pub offset: u32,
        /// Byte stride between rows of the plane.
        pub stride: u32,
    }

    /// One presented frame: a pool image the engine has just rendered.
    ///
    /// `acquire` is a sync file that signals when the engine's submission
    /// completing the image executes on the GPU — the host must not read
    /// the planes until it has signalled. [`DmabufFrame::release`] hands
    /// the image back: the engine reuses it only after the host's own
    /// release sync file has signalled, with no CPU wait. A frame dropped
    /// without `release` is never reused — the bounded pool holds its
    /// pressure on the surface.
    #[derive(Debug)]
    pub struct DmabufFrame {
        /// The image's colour planes — one for the RGBA formats this
        /// target exports.
        pub planes: Vec<DmabufPlane>,
        /// The DRM fourcc (`DRM_FORMAT_*`) of the colour image.
        pub fourcc: u32,
        /// The `DRM_FORMAT_MOD_*` modifier the image was created with —
        /// the driver's negotiated value, never an assumption.
        pub modifier: u64,
        /// Pixel extent of the image.
        pub size: (u32, u32),
        /// The `VkImageLayout` the image is presented in, as `u32`
        /// (`VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL`).
        pub layout: u32,
        /// How the pixels decode into the working space.
        pub color: FrameColor,
        /// How the plane's alpha composes.
        pub alpha: RgbAlpha,
        /// The acquire sync file: signals when the engine's writes are
        /// complete on the GPU. Ownership transfers to the host.
        pub acquire: OwnedFd,
        /// The channel the host's release returns the image on.
        pub(crate) release_to: Sender<crate::render::dmabuf_export::Release>,
        /// The pool image this frame was presented from.
        pub(crate) slot: usize,
    }

    impl DmabufFrame {
        /// Returns the image to the engine's pool once `release` — the
        /// host's own sync file — has signalled that its consumers are
        /// done reading. The engine waits on that fence on the GPU before
        /// writing the image again; it never waits on the CPU.
        ///
        /// On a torn-down surface the release is dropped silently — a
        /// dead pool never reuses the image either way.
        pub fn release(self, release: OwnedFd) {
            let _ = self.release_to.send(crate::render::dmabuf_export::Release {
                slot: self.slot,
                fence: release,
            });
        }
    }

    /// One `(fourcc, modifiers, colour, alpha)` combination the host can
    /// import.
    ///
    /// `modifiers` is the host's declared set for `fourcc`; the engine
    /// picks the first entry the device supports — the explicit modifier
    /// contract the issue fixes. `color`/`alpha` are the presentation
    /// encoding the host expects in the exported pixels.
    #[derive(Debug, Clone)]
    #[non_exhaustive]
    pub struct DmabufFormat {
        /// The DRM fourcc (`DRM_FORMAT_*`) the host imports.
        pub fourcc: u32,
        /// The `DRM_FORMAT_MOD_*` modifiers the host imports, in
        /// preference order.
        pub modifiers: Vec<u64>,
        /// The presentation colour encoding the host expects.
        pub color: OutputColor,
        /// The alpha convention the host expects.
        pub alpha: OutputAlpha,
    }

    impl DmabufFormat {
        /// Declares one importable `(fourcc, modifiers, colour, alpha)`
        /// combination; `modifiers` is in preference order.
        #[must_use]
        pub const fn new(
            fourcc: u32,
            modifiers: Vec<u64>,
            color: OutputColor,
            alpha: OutputAlpha,
        ) -> Self {
            Self {
                fourcc,
                modifiers,
                color,
                alpha,
            }
        }
    }

    /// A surface presented through a pool of exportable dma-buf images.
    ///
    /// `new` returns the target and the frame receiver the host drains:
    /// every [`DmabufFrame`] the engine presents lands on it. `.formats`
    /// declares what the host can import — required, in preference
    /// order; the engine picks the first entry the device supports and
    /// fails the surface with `UnsupportedTarget` when none works.
    /// `.pool` bounds the image count (default three — the Android
    /// `AHardwareBuffer` precedent).
    #[derive(Debug)]
    pub struct DmabufTarget {
        pub(crate) size: (u32, u32),
        pub(crate) sink: Sender<DmabufFrame>,
        pub(crate) formats: Vec<DmabufFormat>,
        pub(crate) refresh: cherenkov::RefreshRange,
        pub(crate) pool_size: usize,
    }

    impl DmabufTarget {
        /// Presents at `size` device pixels; returns the host's frame
        /// receiver.
        #[must_use]
        pub fn new(size: (u32, u32)) -> (Self, Receiver<DmabufFrame>) {
            let (sink, frames) = std::sync::mpsc::channel();
            (
                Self {
                    size,
                    sink,
                    formats: Vec::new(),
                    refresh: cherenkov::DEFAULT_REFRESH,
                    pool_size: 3,
                },
                frames,
            )
        }

        /// Declares the `(fourcc, modifiers, colour)` combinations the
        /// host can import, in preference order.
        #[must_use]
        pub fn formats(mut self, formats: impl Into<Vec<DmabufFormat>>) -> Self {
            self.formats = formats.into();
            self
        }

        /// Sets the refresh range for backend animation and presentation
        /// retries.
        ///
        /// # Panics
        /// When the range is empty or includes zero.
        #[must_use]
        pub fn rate(mut self, rate: cherenkov::RefreshRange) -> Self {
            assert!(
                *rate.start() > 0 && !rate.is_empty(),
                "refresh range must be positive and ordered"
            );
            self.refresh = rate;
            self
        }

        /// Bounds the export pool: with every image out with the host the
        /// surface waits for a release rather than allocating.
        ///
        /// # Panics
        /// When `images` is zero.
        #[must_use]
        pub fn pool(mut self, images: usize) -> Self {
            assert!(images > 0, "the dma-buf pool must hold an image");
            self.pool_size = images;
            self
        }

        /// The size the pool images are allocated at.
        #[must_use]
        pub const fn size(&self) -> (u32, u32) {
            self.size
        }
    }

    impl From<DmabufTarget> for crate::GpuTarget {
        fn from(target: DmabufTarget) -> Self {
            Self::Dmabuf(target)
        }
    }
}

/// Android interop: presenting on system compositor planes (#90).
#[cfg(target_os = "android")]
pub mod android {
    pub use crate::render::surface_control::ffi::{ASurfaceControl, CreateFailed, SurfaceControl};
    pub use crate::render::surface_control::planes::HostedSurface;

    /// A surface realized as child surface controls of a host's parent
    /// surface control.
    ///
    /// The engine's own content is presented on plane buffers the engine
    /// renders into, and eligible external frames are promoted to planes of
    /// their own; every plane of a frame changes in one transaction. The
    /// parent's coordinate space is the surface's device pixel space, with
    /// its origin at the surface's top-left corner. Requires a Vulkan device
    /// with `AHardwareBuffer` import and sync-fence export.
    #[derive(Debug)]
    pub struct SurfaceControlTarget {
        pub(crate) parent: SurfaceControl,
        pub(crate) size: (u32, u32),
        pub(crate) transparent: bool,
        pub(crate) refresh: cherenkov::RefreshRange,
    }

    impl SurfaceControlTarget {
        /// Presents under `parent` at `size` device pixels.
        #[must_use]
        pub const fn new(parent: SurfaceControl, size: (u32, u32)) -> Self {
            Self {
                parent,
                size,
                transparent: false,
                refresh: cherenkov::DEFAULT_REFRESH,
            }
        }

        /// Lets the host's content show through transparent pixels: the
        /// bottom plane blends premultiplied instead of being opaque.
        #[must_use]
        pub const fn transparent(mut self, transparent: bool) -> Self {
            self.transparent = transparent;
            self
        }

        /// Sets the refresh range for backend animation and presentation
        /// retries.
        ///
        /// # Panics
        /// When the range is empty or includes zero.
        #[must_use]
        pub fn rate(mut self, rate: cherenkov::RefreshRange) -> Self {
            assert!(
                *rate.start() > 0 && !rate.is_empty(),
                "refresh range must be positive and ordered"
            );
            self.refresh = rate;
            self
        }

        /// The size the planes are allocated at.
        #[must_use]
        pub const fn size(&self) -> (u32, u32) {
            self.size
        }
    }

    impl From<SurfaceControlTarget> for crate::GpuTarget {
        fn from(target: SurfaceControlTarget) -> Self {
            Self::SurfaceControl(target)
        }
    }
}

/// Apple interop: system layers hosted on the engine's planes.
#[cfg(target_vendor = "apple")]
pub mod apple {
    pub use crate::render::planes::apple::HostedLayer;
}

/// Apple interop: importing Metal resources onto the shared device.
#[cfg(target_vendor = "apple")]
pub mod metal {
    /// Wraps an `MTLTexture` as a `wgpu::Texture` on the engine's device, for
    /// use as an [`ExternalFrame`](super::ExternalFrame) plane.
    ///
    /// The texture is imported in place through wgpu-hal on the shared
    /// device — there is no copy and no conversion texture. The `MTLTexture`
    /// itself stays owned by the caller (typically an `IOSurface` plane made
    /// with `-[MTLDevice newTextureWithDescriptor:iosurface:plane:]` or a
    /// `CVMetalTextureCache` texture); the returned `wgpu::Texture` retains
    /// it until dropped, which the engine's frame leases tie to the
    /// retained frame's lifetime.
    ///
    /// `format` must describe the texels the `MTLTexture` holds exactly — a
    /// format the frame contract accepts, in the plane's role.
    ///
    /// # Safety
    /// `raw` must be a live `MTLTexture` created on the same `MTLDevice` the
    /// `wgpu::Device` wraps, or on another device in its peer group, and
    /// `format` must be byte-compatible with its pixel format. The texture
    /// must stay alive and unwritten-except-by-the-producer for as long as a
    /// frame referencing it can be in flight.
    ///
    /// # Panics
    /// When the `MTLTexture`'s dimensions do not fit `u32`.
    #[must_use]
    pub unsafe fn import_texture(
        device: &wgpu::Device,
        raw: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLTexture>>,
        format: wgpu::TextureFormat,
    ) -> wgpu::Texture {
        use objc2_metal::MTLTexture;
        let extent = wgpu::Extent3d {
            width: u32::try_from(raw.width()).expect("plane width fits u32"),
            height: u32::try_from(raw.height()).expect("plane height fits u32"),
            depth_or_array_layers: 1,
        };
        // SAFETY: the caller's contract guarantees `raw` is a live
        // `MTLTexture` on this device's `MTLDevice` or a peer of it, and
        // `format` is byte-compatible with its pixel format; the 2D,
        // single-mip, single-sample extent mirrors the texture's own.
        let hal_texture = unsafe {
            wgpu::hal::metal::Device::texture_from_raw(
                raw,
                format,
                objc2_metal::MTLTextureType::Type2D,
                1,
                1,
                extent.into(),
                None,
            )
        };
        // SAFETY: `hal_texture` was just wrapped from `raw` for this device,
        // the descriptor repeats its format, extent, mip level and sample
        // count exactly, and the producer's existing contents make
        // `TextureUses::RESOURCE` its actual state.
        unsafe {
            device.create_texture_from_hal::<wgpu::hal::metal::Api>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("external frame plane"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::wgt::TextureUses::RESOURCE,
            )
        }
    }
}

/// Browser interop: importing foreign `GPUTexture`s onto the shared device.
///
/// A producer on the owning JS thread — a video element, a web view, another
/// wgpu or dawn client — holds a `GPUTexture` it wants sampled as an
/// [`ExternalFrame`](super::ExternalFrame) RGB plane without a copy.
/// [`import_texture`] wraps the handle through
/// `wgpu::Device::create_texture_from_webgpu_handle`, validates the provider
/// contract by reflection, and ties the producer's lease to the wrapper's
/// retirement so it is returned only after retained and submitted uses have
/// finished.
///
/// Everything here stays on the JS thread that owns the objects: the engine's
/// local executor runs the future, the wrapper's drop callback runs on the
/// same thread, and nothing is boxed behind an unsafe `Send`.
#[cfg(target_arch = "wasm32")]
pub mod web {
    use super::{InvalidFrame, wgpu};
    use wasm_bindgen::JsCast;
    use wgpu::webgpu;

    /// A foreign `GPUTexture` offered for retained, zero-copy import.
    ///
    /// `texture` is the producer's `GPUTexture` handle — a
    /// `web_sys::GpuTexture` from the producer's own `web-sys` dependency
    /// wraps the same JS object as `wgpu::webgpu::GpuTexture`; convert with
    /// `wasm_bindgen::JsCast::unchecked_into`. `device` is the `GPUDevice`
    /// that created it, supplied as the identity token for the owning-device
    /// check. `release` is the producer's lease hook, run exactly once when
    /// the engine is done with the texture.
    ///
    /// The provider contract: `texture` must be a live, single-sample 2D
    /// `GPUTexture` with one mip level, `TEXTURE_BINDING` usage and a format
    /// the RGB plane role accepts (`rgba8unorm`, `bgra8unorm` or
    /// `rgba16float`), created on `device`, with contents immutable and
    /// lifetime guaranteed through every retained and in-flight use —
    /// including submissions already on the queue, which the WebGPU
    /// implementation keeps alive while they execute.
    ///
    /// Sources that cannot meet that contract are rejected, never copied: a
    /// `GPUExternalTexture`, or a producer that cannot promise immutable
    /// storage for the lease period. Transient handles such as a context's
    /// current canvas texture cannot be detected — they satisfy every
    /// reflected check — so they are excluded by the contract and must not
    /// be offered.
    pub struct WebTexture {
        /// The producer's `GPUTexture` handle.
        pub texture: webgpu::GpuTexture,
        /// The `GPUDevice` that created `texture`.
        pub device: webgpu::GpuDevice,
        /// The producer's release hook; see [`WebTextureLease`].
        pub release: WebTextureLease,
    }

    /// The raw web handles do not format.
    impl std::fmt::Debug for WebTexture {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("WebTexture").finish_non_exhaustive()
        }
    }

    /// The producer's ownership handoff for one imported texture.
    ///
    /// The hook always runs exactly once after [`import_texture`] consumes
    /// the lease: immediately on rejection, or when the returned
    /// `wgpu::Texture`'s last clone is dropped — engine slot replacement,
    /// detach, surface or engine teardown. It is the producer's single
    /// disposal point; typical bodies destroy the texture or return it to a
    /// pool. While the lease is outstanding the producer must keep the
    /// texture alive and immutable.
    pub struct WebTextureLease(Box<dyn FnOnce() + 'static>);

    /// The hook is opaque.
    impl std::fmt::Debug for WebTextureLease {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("WebTextureLease").finish_non_exhaustive()
        }
    }

    impl WebTextureLease {
        /// A lease released by `release`.
        pub fn new(release: impl FnOnce() + 'static) -> Self {
            Self(Box::new(release))
        }
        /// The hook installed as the wgpu wrapper's drop callback.
        fn into_callback(self) -> webgpu::DropCallback {
            self.0
        }
        /// Rejection: the hook runs now rather than at retirement.
        fn fire(self) {
            (self.0)();
        }
    }

    /// Why a [`WebTexture`] was rejected.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
    pub enum InvalidWebTexture {
        /// `texture` is not a `GPUTexture` — a `GPUExternalTexture`, an
        /// ordinary `GPUTextureView`, or any other handle.
        #[error("the handle is not a GPUTexture")]
        NotATexture,
        /// `device` is not the `GPUDevice` the engine's `wgpu::Device` wraps.
        #[error("the texture's device token is not the shared GPUDevice")]
        DeviceMismatch,
        /// The engine's `wgpu::Device` is not backed by WebGPU.
        #[error("the engine device is not a WebGPU device")]
        NotWebGpu,
        /// The texture violates the shared plane contract (format,
        /// geometry, dimensions or usage).
        #[error(transparent)]
        Contract(#[from] InvalidFrame),
        /// A submission referencing the texture failed validation: the
        /// texture was destroyed, belongs to another device than its token
        /// claims, or is otherwise unusable for retained sampling.
        #[error("the texture cannot be used in a submission on this device")]
        Unusable,
    }

    /// A reflected `GPUTexture` attribute.
    fn attr(texture: &webgpu::GpuTexture, name: &str) -> wasm_bindgen::JsValue {
        js_sys::Reflect::get(texture.as_ref(), &name.into()).unwrap_or_default()
    }

    /// The reflected provider contract, checked before any wrap:
    /// `(format, size, usage)` for the wrapper's descriptor. The device
    /// token must identify the `GPUDevice` that created the texture —
    /// a wrong-token lie is caught by the submission probe, since the
    /// token is the only device identity JavaScript exposes.
    fn contract(
        gpu_device: Option<&webgpu::GpuDevice>,
        texture: &webgpu::GpuTexture,
        token: &webgpu::GpuDevice,
    ) -> Result<(wgpu::TextureFormat, wgpu::Extent3d, u32), InvalidWebTexture> {
        let gpu_device = gpu_device.ok_or(InvalidWebTexture::NotWebGpu)?;
        if !texture.has_type::<webgpu::GpuTexture>() {
            return Err(InvalidWebTexture::NotATexture);
        }
        if !js_sys::Object::is(gpu_device.as_ref(), token.as_ref()) {
            return Err(InvalidWebTexture::DeviceMismatch);
        }
        let format = match attr(texture, "format").as_string().as_deref() {
            Some("rgba8unorm") => wgpu::TextureFormat::Rgba8Unorm,
            Some("bgra8unorm") => wgpu::TextureFormat::Bgra8Unorm,
            Some("rgba16float") => wgpu::TextureFormat::Rgba16Float,
            _ => return Err(InvalidFrame::PlaneFormat.into()),
        };
        let size = wgpu::Extent3d {
            width: texture.width(),
            height: texture.height(),
            depth_or_array_layers: 1,
        };
        if attr(texture, "dimension").as_string().as_deref() != Some("2d")
            || texture.depth_or_array_layers() != 1
            || texture.mip_level_count() != 1
            || texture.sample_count() != 1
        {
            return Err(InvalidFrame::PlaneGeometry.into());
        }
        if size.width == 0 || size.height == 0 {
            return Err(InvalidFrame::PlaneDimensions.into());
        }
        let usage = texture.usage();
        if usage & wgpu::TextureUsages::TEXTURE_BINDING.bits() == 0 {
            return Err(InvalidFrame::PlaneUsage.into());
        }
        Ok((format, size, usage))
    }

    /// Wraps a foreign `GPUTexture` on the engine's device for use as an
    /// [`ExternalFrame`](super::ExternalFrame) RGB plane.
    ///
    /// The handle is wrapped once, in place, through
    /// `Device::create_texture_from_webgpu_handle`: no pixels upload, no
    /// copy runs and no conversion texture is allocated. Cloning the
    /// returned texture shares the one wrapper — and the one lease — across
    /// layer attachments; the lease's release hook runs when the last clone
    /// is dropped, after every retained and submitted use has finished.
    ///
    /// The provider contract is validated by reflection (format, size,
    /// usage, mip and sample count, owning device token) and by a
    /// submission probe: an empty pass bound to the wrapper inside a
    /// validation error scope is the only place a destroyed or cross-device
    /// `GPUTexture` provably errors, since JavaScript exposes no liveness
    /// flag and creation-time calls accept destroyed textures. A rejection
    /// releases the lease before returning.
    ///
    /// `queue` must be the engine's submission queue, so the probe orders
    /// behind the producer's pending work on the same device.
    ///
    /// # Errors
    /// [`InvalidWebTexture`] when the provider contract fails or the texture
    /// proves unusable in a submission; the lease's release hook has already
    /// run by the time the error is returned.
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    pub async fn import_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: WebTexture,
    ) -> Result<wgpu::Texture, InvalidWebTexture> {
        let WebTexture {
            texture,
            device: token,
            release,
        } = source;
        let (format, size, usage) = match contract(device.as_webgpu(), &texture, &token) {
            Ok(reflected) => reflected,
            Err(error) => {
                release.fire();
                return Err(error);
            }
        };
        let wrapped = device.create_texture_from_webgpu_handle(
            texture,
            &wgpu::TextureDescriptor {
                label: Some("external frame plane"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::from_bits_retain(usage),
                view_formats: &[],
            },
            Some(release.into_callback()),
        );
        let probe_error = {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("external frame probe"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("external frame probe"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &wrapped.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                }],
            });
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                pass.set_bind_group(0, &group, &[]);
            }
            queue.submit([encoder.finish()]);
            scope.pop().await
        };
        if probe_error.is_some() {
            drop(wrapped);
            return Err(InvalidWebTexture::Unusable);
        }
        Ok(wrapped)
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameColor, YuvRange};

    /// BT.2020 PQ video ships studio-range codes: `FrameColor::BT2020_PQ`
    /// names that convention — the former "full-range" doc was the typo,
    /// not a value to align (#2109).
    #[test]
    fn bt2020_pq_is_studio_range() {
        assert_eq!(FrameColor::BT2020_PQ.range, YuvRange::Video);
    }
}
