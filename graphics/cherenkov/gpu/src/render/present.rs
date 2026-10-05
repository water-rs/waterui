//! Window presentation: blits a surface's f16 target onto its swapchain.
//!
//! The swapchain's format/colour-space pair is negotiated from the
//! surface's advertised capabilities — [`select_output`], the output
//! negotiation of #98 — never a fixed sRGB choice. The selected pair
//! picks the transfer encoding `present.wgsl` writes; headroom arrives
//! per frame through [`Display`](cherenkov::Display)'s `headroom` and is
//! clamped to what the destination can carry.

use rustc_hash::FxHashMap;

use cherenkov::{RenderError, SurfaceError};

use crate::DisplaySync;

use super::{bindings, layout_entries, shaders::ShaderDelivery};

/// SDR white in nits — the BT.2408 reference white the absolute and
/// relative HDR encodings calibrate against: working-space `1.0` presents
/// at 203 nits. `cherenkov`'s PQ/HLG decode contract uses the same value.
pub const REFERENCE_WHITE_NITS: f32 = 203.0;

/// The headroom a BT.2100 PQ output carries: 10000 nits of PQ range at
/// the 203-nit reference white.
const PQ_HEADROOM: f32 = 10000.0 / REFERENCE_WHITE_NITS;

/// The headroom a BT.2100 HLG output carries: the 1000-nit nominal peak
/// at the 203-nit reference white.
const HLG_HEADROOM: f32 = 1000.0 / REFERENCE_WHITE_NITS;

/// Destination primaries the present pass converts to (#98).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DestinationPrimaries {
    /// BT.709 / sRGB.
    Srgb,
    /// Display P3 — the working space's primaries, no conversion.
    DisplayP3,
    /// BT.2020/2100, the PQ and HLG signal gamut.
    Bt2100,
}

/// The destination transfer function the present pass encodes (#98).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferEncoding {
    /// Linear — float destinations keep the premultiplied linear values.
    Linear,
    /// The sRGB OETF — hardware applies it on `*Srgb` formats, the shader
    /// encodes for plain unorm formats.
    Srgb,
    /// The sRGB OETF extended sign-symmetrically beyond `[0, 1]` — the
    /// `ExtendedSrgb`/`ExtendedDisplayP3` wire format.
    ExtendedSrgb,
    /// SMPTE ST 2084 (PQ).
    Pq,
    /// BT.2100 hybrid log-gamma.
    Hlg,
}

/// Why a selection resolved the way it did — reported, never silent (#98).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionReason {
    /// The backend table's preferred HDR pair was advertised.
    PreferredHdr,
    /// A wide-gamut SDR pair (`DisplayP3`) was advertised where no
    /// extended or HDR pair was.
    WideGamutSdr,
    /// sRGB SDR: the explicit negotiation result when nothing wider is
    /// advertised.
    Sdr,
    /// The colour space the host required through
    /// `WindowTarget::require_color_space`.
    Required,
}

/// A negotiated output configuration: the advertised (format, colour
/// space) pair the swapchain is configured with and everything the
/// presentation pass needs to encode for it (#98).
#[derive(Clone, Debug)]
pub struct OutputSelection {
    /// The swapchain's texture format.
    pub format: wgpu::TextureFormat,
    /// The configured colour space — resolved, never `Auto` unless the
    /// surface advertised no explicit pair at all.
    pub color_space: wgpu::SurfaceColorSpace,
    /// The primaries the present pass converts to.
    pub primaries: DestinationPrimaries,
    /// The transfer the present pass encodes.
    pub transfer: TransferEncoding,
    /// The composite alpha mode the swapchain is configured with.
    pub alpha_mode: wgpu::CompositeAlphaMode,
    /// The present mode the swapchain is configured with: what the host's
    /// [`DisplaySync`](crate::DisplaySync) resolved to on this surface
    /// (#214).
    pub present_mode: wgpu::PresentMode,
    /// The display's reported tone-map headroom when the selection was
    /// made (`Surface::display_hdr_info`). `None` where the query is
    /// unavailable — on Metal it answers only from the main thread, so
    /// the live value arrives through the [`DisplayProbe`] instead.
    /// Unknown is never guessed as SDR.
    pub reported_headroom: Option<f32>,
    /// The headroom the destination can carry: `1` for the SDR spaces,
    /// `REFERENCE_WHITE_NITS`-calibrated ceilings for PQ and HLG,
    /// unbounded (`f32::MAX`) for the extended spaces.
    pub tone_map_ceiling: f32,
    /// The SDR-white calibration in nits (BT.2408).
    pub reference_white_nits: f32,
    /// Why this pair was selected.
    pub reason: SelectionReason,
}

impl OutputSelection {
    /// The effective tone-map headroom for a display `headroom`: the
    /// conservative `1` when the display's is unknown, never more than
    /// the destination carries. Headroom is presentation-time data — a
    /// display change never reselects the swapchain.
    #[must_use]
    pub const fn effective_headroom(&self, headroom: f32) -> f32 {
        headroom.clamp(0.0, self.tone_map_ceiling)
    }

    /// The [`OutputColor`] the selection maps to for the present pass.
    #[must_use]
    pub const fn output_color(&self) -> OutputColor {
        match (self.primaries, self.transfer) {
            (DestinationPrimaries::DisplayP3, TransferEncoding::Srgb) => OutputColor::DisplayP3,
            (DestinationPrimaries::DisplayP3, TransferEncoding::Linear) => {
                OutputColor::LinearDisplayP3
            }
            (DestinationPrimaries::Srgb, TransferEncoding::Linear) => {
                OutputColor::ExtendedSrgbLinear
            }
            (DestinationPrimaries::Srgb, TransferEncoding::ExtendedSrgb) => {
                OutputColor::ExtendedSrgb
            }
            (DestinationPrimaries::DisplayP3, TransferEncoding::ExtendedSrgb) => {
                OutputColor::ExtendedDisplayP3
            }
            (DestinationPrimaries::Bt2100, TransferEncoding::Pq) => OutputColor::Bt2100Pq,
            (DestinationPrimaries::Bt2100, TransferEncoding::Hlg) => OutputColor::Bt2100Hlg,
            // Every other (primaries, transfer) pair resolves to the
            // SDR sRGB encoding — a pair the negotiation never produces.
            _ => OutputColor::Srgb,
        }
    }
}

/// Whether `caps` advertises `format` usable with `color_space` — a
/// supported *pair*, never a Cartesian product of the lists.
fn advertised(
    caps: &wgpu::SurfaceCapabilities,
    format: wgpu::TextureFormat,
    color_space: wgpu::SurfaceColorSpace,
) -> bool {
    let Some(spaces) = color_space.to_color_spaces() else {
        return caps.formats.contains(&format);
    };
    caps.format_capabilities
        .iter()
        .any(|fc| fc.format == format && fc.color_spaces.contains(spaces))
}

/// The first format the surface advertises for `color_space`, in the
/// surface's own preference order (`format_capabilities` is ordered like
/// `formats`).
fn first_for_space(
    caps: &wgpu::SurfaceCapabilities,
    color_space: wgpu::SurfaceColorSpace,
) -> Option<wgpu::TextureFormat> {
    let spaces = color_space.to_color_spaces()?;
    caps.format_capabilities
        .iter()
        .find(|fc| fc.color_spaces.contains(spaces))
        .map(|fc| fc.format)
}

/// One selection candidate: an exact pair, or a colour space with the
/// format left to the surface's preference order.
#[derive(Clone, Copy)]
enum Want {
    Pair(wgpu::TextureFormat, wgpu::SurfaceColorSpace),
    AnyFormat(wgpu::SurfaceColorSpace),
}

/// The backend's preference order of candidates — the plan of record's
/// per-platform tables (#98). An `Auto` tail replicates the historical
/// `formats.first()` fallback for surfaces that advertise no explicit
/// colour space.
fn preferred_candidates(backend: wgpu::Backend) -> Vec<Want> {
    use wgpu::SurfaceColorSpace as Cs;
    use wgpu::TextureFormat as F;
    let f16 = F::Rgba16Float;
    let ten = F::Rgb10a2Unorm;
    match backend {
        wgpu::Backend::Metal => vec![
            Want::Pair(f16, Cs::ExtendedDisplayP3),
            Want::Pair(f16, Cs::ExtendedSrgbLinear),
            Want::Pair(f16, Cs::DisplayP3),
            Want::AnyFormat(Cs::DisplayP3),
            Want::AnyFormat(Cs::Srgb),
        ],
        wgpu::Backend::Vulkan => vec![
            Want::Pair(f16, Cs::ExtendedSrgbLinear),
            Want::Pair(ten, Cs::Bt2100Pq),
            Want::Pair(f16, Cs::Bt2100Pq),
            Want::AnyFormat(Cs::DisplayP3),
            Want::AnyFormat(Cs::Srgb),
        ],
        wgpu::Backend::Dx12 => vec![
            Want::Pair(f16, Cs::ExtendedSrgbLinear),
            Want::Pair(ten, Cs::Bt2100Pq),
            Want::Pair(f16, Cs::Bt2100Pq),
            Want::AnyFormat(Cs::Srgb),
        ],
        wgpu::Backend::BrowserWebGpu => vec![
            Want::Pair(f16, Cs::ExtendedDisplayP3),
            Want::Pair(f16, Cs::ExtendedSrgb),
            Want::AnyFormat(Cs::DisplayP3),
            Want::AnyFormat(Cs::Srgb),
        ],
        _ => vec![Want::AnyFormat(Cs::Srgb)],
    }
}

/// The format preference order for an explicitly required colour space.
const fn required_format_order(
    color_space: wgpu::SurfaceColorSpace,
) -> &'static [wgpu::TextureFormat] {
    use wgpu::SurfaceColorSpace as Cs;
    use wgpu::TextureFormat as F;
    match color_space {
        Cs::Bt2100Pq | Cs::Bt2100Hlg => &[F::Rgb10a2Unorm, F::Rgba16Float],
        Cs::ExtendedSrgbLinear | Cs::ExtendedSrgb | Cs::ExtendedDisplayP3 | Cs::DisplayP3 => {
            &[F::Rgba16Float]
        }
        Cs::Auto | Cs::Srgb => &[],
    }
}

/// The record's fields that follow from the colour space alone.
const fn space_characteristics(
    color_space: wgpu::SurfaceColorSpace,
) -> (DestinationPrimaries, TransferEncoding, f32) {
    use wgpu::SurfaceColorSpace as Cs;
    match color_space {
        Cs::DisplayP3 => (DestinationPrimaries::DisplayP3, TransferEncoding::Srgb, 1.0),
        Cs::ExtendedSrgbLinear => (
            DestinationPrimaries::Srgb,
            TransferEncoding::Linear,
            f32::MAX,
        ),
        Cs::ExtendedSrgb => (
            DestinationPrimaries::Srgb,
            TransferEncoding::ExtendedSrgb,
            f32::MAX,
        ),
        Cs::ExtendedDisplayP3 => (
            DestinationPrimaries::DisplayP3,
            TransferEncoding::ExtendedSrgb,
            f32::MAX,
        ),
        Cs::Bt2100Pq => (
            DestinationPrimaries::Bt2100,
            TransferEncoding::Pq,
            PQ_HEADROOM,
        ),
        Cs::Bt2100Hlg => (
            DestinationPrimaries::Bt2100,
            TransferEncoding::Hlg,
            HLG_HEADROOM,
        ),
        Cs::Auto | Cs::Srgb => (DestinationPrimaries::Srgb, TransferEncoding::Srgb, 1.0),
    }
}

/// What a host asked of a window's swapchain — the `WindowTarget`
/// options output negotiation runs against.
#[derive(Clone, Copy, Debug)]
pub struct OutputRequest {
    /// `WindowTarget::transparent`: a composite alpha mode the compositor
    /// sees through.
    pub transparent: bool,
    /// `WindowTarget::require_color_space`: the required colour space, or
    /// `None` to negotiate the best advertised pair.
    pub color_space: Option<wgpu::SurfaceColorSpace>,
    /// `WindowTarget::display_sync`: how presentation is paced against
    /// the display.
    pub sync: DisplaySync,
}

/// The present mode `sync` resolves to on a surface advertising `caps`:
/// the first of its candidates the surface offers (#214).
fn select_present_mode(
    caps: &wgpu::SurfaceCapabilities,
    sync: DisplaySync,
) -> Result<wgpu::PresentMode, SurfaceError> {
    let candidates: &[wgpu::PresentMode] = match sync {
        DisplaySync::Synchronized => &[wgpu::PresentMode::Fifo],
        DisplaySync::Unsynchronized => &[wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate],
    };
    candidates
        .iter()
        .copied()
        .find(|mode| caps.present_modes.contains(mode))
        .ok_or_else(|| {
            SurfaceError::UnsupportedTarget(format!(
                "{sync:?} presentation needs one of the present modes {candidates:?}, the surface offers {:?}",
                caps.present_modes
            ))
        })
}

/// Selects the swapchain's (format, colour space) pair and present mode
/// from the surface's advertised capabilities — output negotiation, not a
/// fallback (#98, #214).
///
/// `request.color_space` is the host's explicit colour-space requirement
/// (`WindowTarget::require_color_space`) and `request.sync` its pacing
/// (`WindowTarget::display_sync`); when either cannot be met the surface
/// is `Unsupported`, never silently substituted.
///
/// # Errors
/// [`SurfaceError::UnsupportedTarget`] when the surface offers no present
/// mode for `request.sync`, no transparency-capable alpha mode for a
/// transparent request, the required colour space, or any format at all.
///
/// # Panics
/// If the surface reports no composite alpha mode, which a configurable
/// surface always does.
pub fn select_output(
    caps: &wgpu::SurfaceCapabilities,
    backend: wgpu::Backend,
    request: OutputRequest,
) -> Result<OutputSelection, SurfaceError> {
    let present_mode = select_present_mode(caps, request.sync)?;
    let alpha_mode = if request.transparent {
        [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::PostMultiplied,
        ]
        .into_iter()
        .find(|mode| caps.alpha_modes.contains(mode))
        .ok_or_else(|| {
            SurfaceError::UnsupportedTarget(format!(
                "a transparent window needs a transparency-capable composite alpha mode, the adapter offers {:?}",
                caps.alpha_modes
            ))
        })?
    } else if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
        wgpu::CompositeAlphaMode::Opaque
    } else {
        *caps
            .alpha_modes
            .first()
            .expect("a configurable surface reports at least one alpha mode")
    };
    let (format, color_space, reason) = match request.color_space {
        Some(wgpu::SurfaceColorSpace::Auto) | None => {
            let mut selected = None;
            for want in preferred_candidates(backend) {
                let pair = match want {
                    Want::Pair(format, space) => {
                        advertised(caps, format, space).then_some((format, space))
                    }
                    Want::AnyFormat(space) => {
                        first_for_space(caps, space).map(|format| (format, space))
                    }
                };
                if let Some(pair) = pair {
                    selected = Some(pair);
                    break;
                }
            }
            match selected {
                Some((format, space)) => (
                    format,
                    space,
                    if space.is_hdr() {
                        SelectionReason::PreferredHdr
                    } else if space == wgpu::SurfaceColorSpace::DisplayP3 {
                        SelectionReason::WideGamutSdr
                    } else {
                        SelectionReason::Sdr
                    },
                ),
                // A surface that advertises no explicit colour space —
                // the historical `formats.first()` + `Auto` configuration.
                None => (
                    caps.formats.first().copied().ok_or_else(|| {
                        SurfaceError::UnsupportedTarget(
                            "adapter cannot present to the window".into(),
                        )
                    })?,
                    wgpu::SurfaceColorSpace::Auto,
                    SelectionReason::Sdr,
                ),
            }
        }
        Some(required_space) => {
            let mut format = None;
            for &preferred in required_format_order(required_space) {
                if advertised(caps, preferred, required_space) {
                    format = Some(preferred);
                    break;
                }
            }
            let format = match format {
                Some(format) => format,
                None => first_for_space(caps, required_space).ok_or_else(|| {
                    SurfaceError::UnsupportedTarget(format!(
                        "colour space {required_space:?} is not advertised for this surface"
                    ))
                })?,
            };
            (format, required_space, SelectionReason::Required)
        }
    };
    let (primaries, transfer, tone_map_ceiling) = space_characteristics(color_space);
    Ok(OutputSelection {
        format,
        color_space,
        primaries,
        transfer,
        alpha_mode,
        present_mode,
        reported_headroom: None,
        tone_map_ceiling,
        reference_white_nits: REFERENCE_WHITE_NITS,
        reason,
    })
}

/// The surface handle shared between a window's swapchain and its
/// [`DisplayProbe`]: `Arc` where the probe can be sampled on another
/// thread, `Rc` on wasm32, where the surface is DOM-bound and the engine
/// is single-threaded — `wgpu::Surface` is `!Send`/`!Sync` there.
#[cfg(not(target_arch = "wasm32"))]
type SharedSurface = std::sync::Arc<wgpu::Surface<'static>>;
/// The wasm32 shared surface handle: single-threaded, so `Rc`.
#[cfg(target_arch = "wasm32")]
type SharedSurface = std::rc::Rc<wgpu::Surface<'static>>;

/// A main-thread handle to a window surface's live display state (#98).
///
/// wgpu's `Surface::display_hdr_info` must run on the main thread on
/// Apple; the renderer owns the surface on its own thread, so the probe
/// is delivered to the host through
/// [`WindowTarget::output_probe`](crate::WindowTarget::output_probe).
/// The host samples it when the display changes (or on every presentation
/// tick) and feeds the result back through
/// [`Surface::display`](cherenkov::Surface::display).
pub struct DisplayProbe {
    surface: SharedSurface,
    adapter: wgpu::Adapter,
    backend: wgpu::Backend,
    request: OutputRequest,
}

impl DisplayProbe {
    /// The display's live HDR characteristics. On Apple this must be
    /// called on the main thread; elsewhere any thread is safe.
    #[must_use]
    pub fn display_hdr_info(&self) -> wgpu::DisplayHdrInfo {
        self.surface.display_hdr_info(&self.adapter)
    }

    /// The display's current tone-map headroom multiplier of SDR white —
    /// `None` when the display reports nothing (never guessed as SDR).
    /// Feed a known value into [`Display::headroom`].
    ///
    /// [`Display::headroom`]: cherenkov::Display::headroom
    #[must_use]
    pub fn tone_map_headroom(&self) -> Option<f32> {
        self.display_hdr_info().tone_map_headroom()
    }

    /// Re-enumerates the surface's capabilities and re-runs selection —
    /// the pair a window moved to a different display would be
    /// configured with now.
    ///
    /// # Errors
    /// [`SurfaceError::UnsupportedTarget`] when nothing advertised can be
    /// selected.
    pub fn selection(&self) -> Result<OutputSelection, SurfaceError> {
        let caps = self.surface.get_capabilities(&self.adapter);
        select_output(&caps, self.backend, self.request)
    }
}

impl core::fmt::Debug for DisplayProbe {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DisplayProbe")
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

/// A window's swapchain and its configuration.
#[derive(Debug)]
pub struct WindowSurface {
    surface: SharedSurface,
    config: wgpu::SurfaceConfiguration,
    /// The negotiated output — changes only when a re-selection finds a
    /// different advertised pair (#98).
    selection: OutputSelection,
    /// The backend and host request the selection was made under, kept
    /// for re-selection on display changes.
    backend: wgpu::Backend,
    request: OutputRequest,
}

impl WindowSurface {
    /// Creates and configures the swapchain for `handle` at `size`,
    /// negotiating the output from the surface's advertised capabilities.
    /// `probe` receives the [`DisplayProbe`] the host samples for live
    /// headroom.
    ///
    /// # Errors
    /// [`SurfaceError::UnsupportedTarget`] when wgpu cannot create or the
    /// adapter cannot present to the window — including a required colour
    /// space the surface does not advertise and pacing it cannot present
    /// with.
    #[cfg(not(target_vendor = "apple"))]
    pub fn new(
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        handle: Box<dyn wgpu::WindowHandle>,
        size: (u32, u32),
        request: OutputRequest,
        probe: Option<std::sync::mpsc::Sender<DisplayProbe>>,
    ) -> Result<Self, SurfaceError> {
        let surface = SharedSurface::new(
            instance
                .create_surface(wgpu::SurfaceTarget::Window(handle))
                .map_err(|e| SurfaceError::UnsupportedTarget(format!("window surface: {e}")))?,
        );
        Self::configure(surface, adapter, device, size, request, probe)
    }

    /// Creates and configures the swapchain of a metal layer the engine
    /// owns (a plane surface's part).
    ///
    /// # Errors
    /// [`SurfaceError::UnsupportedTarget`] when wgpu cannot create or the
    /// adapter cannot present to the layer — including a required colour
    /// space the surface does not advertise and pacing it cannot present
    /// with.
    #[cfg(target_vendor = "apple")]
    pub fn from_layer(
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        layer: &objc2_quartz_core::CAMetalLayer,
        size: (u32, u32),
        request: OutputRequest,
        probe: Option<std::sync::mpsc::Sender<DisplayProbe>>,
    ) -> Result<Self, SurfaceError> {
        // SAFETY: the layer is a live `CAMetalLayer`; the surface retains it.
        let surface = SharedSurface::new(
            unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(
                    std::ptr::from_ref(layer).cast_mut().cast(),
                ))
            }
            .map_err(|e| SurfaceError::UnsupportedTarget(format!("metal layer surface: {e}")))?,
        );
        Self::configure(surface, adapter, device, size, request, probe)
    }

    /// Configures `surface` at `size`: output selection, the swapchain and
    /// the host's display probe.
    fn configure(
        surface: SharedSurface,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        size: (u32, u32),
        request: OutputRequest,
        probe: Option<std::sync::mpsc::Sender<DisplayProbe>>,
    ) -> Result<Self, SurfaceError> {
        let backend = adapter.get_info().backend;
        let caps = surface.get_capabilities(adapter);
        let mut selection = select_output(&caps, backend, request)?;
        // `display_hdr_info` answers only on the main thread on Apple;
        // there the host's probe reports instead (#98).
        if backend != wgpu::Backend::Metal {
            selection.reported_headroom = surface.display_hdr_info(adapter).tone_map_headroom();
        }
        let config = wgpu::SurfaceConfiguration {
            color_space: selection.color_space,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: selection.format,
            width: size.0.max(1),
            height: size.1.max(1),
            present_mode: selection.present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: selection.alpha_mode,
            view_formats: Vec::new(),
        };
        surface.configure(device, &config);
        tracing::debug!(
            format = ?selection.format,
            color_space = ?selection.color_space,
            present_mode = ?selection.present_mode,
            reason = ?selection.reason,
            reported_headroom = selection.reported_headroom,
            "swapchain output selected"
        );
        if let Some(probe) = probe {
            let _ = probe.send(DisplayProbe {
                surface: SharedSurface::clone(&surface),
                adapter: adapter.clone(),
                backend,
                request,
            });
        }
        Ok(Self {
            surface,
            config,
            selection,
            backend,
            request,
        })
    }

    /// The output negotiated at (re)selection — diagnostics for hosts
    /// and the bench.
    #[must_use]
    pub const fn selection(&self) -> &OutputSelection {
        &self.selection
    }

    /// Reconfigures the swapchain to `size`.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        self.config.width = size.0.max(1);
        self.config.height = size.1.max(1);
        self.surface.configure(device, &self.config);
    }

    /// Re-runs output negotiation — on a display change the advertised
    /// capabilities may have moved. Reconfigures only when the selected
    /// format/colour-space/alpha/present-mode tuple actually changes (#98).
    ///
    /// # Errors
    /// [`SurfaceError::UnsupportedTarget`] when the surface no longer
    /// advertises anything the host's request can be met with; the
    /// swapchain is left as it was, never reconfigured to a substitute.
    pub fn reselect(
        &mut self,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
    ) -> Result<(), SurfaceError> {
        let caps = self.surface.get_capabilities(adapter);
        let mut selection = select_output(&caps, self.backend, self.request)?;
        if self.backend != wgpu::Backend::Metal {
            selection.reported_headroom =
                self.surface.display_hdr_info(adapter).tone_map_headroom();
        }
        if selection.format == self.selection.format
            && selection.color_space == self.selection.color_space
            && selection.alpha_mode == self.selection.alpha_mode
            && selection.present_mode == self.selection.present_mode
        {
            self.selection = selection;
            return Ok(());
        }
        tracing::debug!(
            format = ?selection.format,
            color_space = ?selection.color_space,
            present_mode = ?selection.present_mode,
            reason = ?selection.reason,
            "swapchain output reselected"
        );
        self.config.format = selection.format;
        self.config.color_space = selection.color_space;
        self.config.alpha_mode = selection.alpha_mode;
        self.config.present_mode = selection.present_mode;
        self.surface.configure(device, &self.config);
        self.selection = selection;
        Ok(())
    }

    /// Acquires the next swapchain image, reconfiguring once when the
    /// swapchain is outdated or lost. `None` skips the frame: the window is
    /// occluded or the acquire timed out.
    fn acquire(&self, device: &wgpu::Device) -> Result<Option<wgpu::SurfaceTexture>, RenderError> {
        use wgpu::CurrentSurfaceTexture as Current;
        let acquired = match self.surface.get_current_texture() {
            Current::Outdated | Current::Lost => {
                self.surface.configure(device, &self.config);
                self.surface.get_current_texture()
            }
            other => other,
        };
        match acquired {
            Current::Success(texture) | Current::Suboptimal(texture) => Ok(Some(texture)),
            Current::Timeout | Current::Occluded => Ok(None),
            Current::Outdated | Current::Lost => Err(RenderError::Render(
                "swapchain acquire failed after reconfiguration".into(),
            )),
            Current::Validation => Err(RenderError::Render("swapchain acquire: validation".into())),
        }
    }
}

/// The present pipelines, one per swapchain format seen.
/// Presents retained engine textures into host-owned textures.
#[derive(Debug)]
pub struct Presenter {
    module: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    pipelines: FxHashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    /// `{ encode, alpha, headroom, pad }`: see `Present` in
    /// `present.wgsl`. Written per call — the headroom follows the
    /// display every frame (#97).
    uniform: wgpu::Buffer,
}

impl Presenter {
    /// Creates the shared present state.
    #[must_use]
    pub fn new(device: &wgpu::Device, delivery: ShaderDelivery) -> Self {
        let module = delivery.present_module(device);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present"),
            entries: &layout_entries(bindings::PRESENT_GROUP0),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("present"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..wgpu::SamplerDescriptor::default()
        });
        let uniform = Self::uniform(device);
        Self {
            module,
            layout,
            pipeline_layout,
            sampler,
            pipelines: FxHashMap::default(),
            uniform,
        }
    }

    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        self.pipelines.entry(format).or_insert_with(|| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("present"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.module,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.module,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    /// Blits `source` onto the window's next swapchain image and presents
    /// it. `headroom` is the display's declared HDR headroom for this
    /// frame.
    ///
    /// # Errors
    /// [`RenderError`] when the swapchain image cannot be acquired.
    pub fn present(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        window: &WindowSurface,
        source: &wgpu::TextureView,
        headroom: f32,
    ) -> Result<bool, RenderError> {
        let Some(frame) = self.prepare(device, queue, window, source, headroom)? else {
            return Ok(false);
        };
        queue.present(frame);
        Ok(true)
    }

    /// Encodes into an owned drawable, which the compositor presents in
    /// the same main-thread transaction as its plane hierarchy.
    ///
    /// # Errors
    /// Returns a render error when the window cannot acquire or configure
    /// its drawable.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        window: &WindowSurface,
        source: &wgpu::TextureView,
        headroom: f32,
    ) -> Result<Option<wgpu::SurfaceTexture>, RenderError> {
        let Some(frame) = window.acquire(device)? else {
            return Ok(None);
        };
        let alpha = match window.config.alpha_mode {
            wgpu::CompositeAlphaMode::PreMultiplied | wgpu::CompositeAlphaMode::Inherit => {
                OutputAlpha::Premultiplied
            }
            // Core Animation composites a non-opaque layer's contents as
            // premultiplied; wgpu's Metal backend offers exactly that mode,
            // under the name `PostMultiplied`.
            #[cfg(target_vendor = "apple")]
            wgpu::CompositeAlphaMode::PostMultiplied => OutputAlpha::Premultiplied,
            #[cfg(not(target_vendor = "apple"))]
            wgpu::CompositeAlphaMode::PostMultiplied => OutputAlpha::Straight,
            wgpu::CompositeAlphaMode::Auto | wgpu::CompositeAlphaMode::Opaque => {
                OutputAlpha::Opaque
            }
        };
        self.texture(
            device,
            queue,
            source,
            TextureOutput {
                texture: &frame.texture,
                color: window.selection.output_color(),
                alpha,
                headroom: window.selection.effective_headroom(headroom),
            },
        );
        Ok(Some(frame))
    }

    /// Composites an engine texture into a native texture on the same device.
    /// `source` contains premultiplied linear Display P3. `color` describes
    /// the destination's color space; sRGB texture formats encode in hardware.
    /// The destination must be a single-sampled, renderable 2D texture on this
    /// device. The blit replaces its contents, scaling the source to fit.
    ///
    /// # Panics
    /// If linear Display P3 output is requested for an sRGB texture format,
    /// or the textures violate wgpu's attachment and sampling requirements.
    pub fn texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &wgpu::TextureView,
        output: TextureOutput<'_>,
    ) {
        self.texture_timed(device, queue, source, output, None);
    }

    /// [`Self::texture`], writing GPU timestamps around the render pass —
    /// the presentation cost probe the cross-engine bench's `present-cost`
    /// mode uses (#96). The device must have `Features::TIMESTAMP_QUERY`.
    ///
    /// # Panics
    /// As [`Self::texture`].
    pub fn texture_timed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &wgpu::TextureView,
        output: TextureOutput<'_>,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        queue.write_buffer(&self.uniform, 0, &Self::uniform_bytes(&output));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("present"),
        });
        let uniform = self.uniform.clone();
        self.encode(device, &mut encoder, source, output, &uniform, timestamps);
        queue.submit([encoder.finish()]);
        crate::diag::submit(device, queue, "present");
    }

    /// A 16-byte uniform buffer for [`Self::encode`], written with
    /// [`Self::uniform_bytes`] before the submission that reads it. Several
    /// blits in one submission each need their own.
    #[must_use]
    pub(crate) fn uniform(device: &wgpu::Device) -> wgpu::Buffer {
        crate::diag::create(device, "present uniform", 16);
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("present uniform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// The uniform contents a blit into `output` reads: `{ encode, alpha,
    /// headroom, pad }`, see `Present` in `present.wgsl`.
    ///
    /// # Panics
    /// If linear Display P3 output is requested for an sRGB texture format.
    #[must_use]
    pub(crate) fn uniform_bytes(output: &TextureOutput<'_>) -> [u8; 16] {
        let format = output.texture.format();
        assert!(
            !matches!(
                output.color,
                OutputColor::LinearDisplayP3
                    | OutputColor::ExtendedSrgbLinear
                    | OutputColor::ExtendedSrgb
                    | OutputColor::ExtendedDisplayP3
            ) || !format.is_srgb(),
            "extended output requires a non-sRGB texture format"
        );
        // The `present.wgsl` `encode` codes: 0/1 sRGB SDR (hw/sw),
        // 2 linear P3, 3/4 Display P3 SDR (sw/hw), 5 scRGB linear,
        // 6 extended sRGB, 7 extended P3, 8 PQ, 9 HLG.
        let encode: u32 = match output.color {
            OutputColor::Srgb => u32::from(!format.is_srgb()),
            OutputColor::LinearDisplayP3 => 2,
            OutputColor::DisplayP3 => 3 + u32::from(format.is_srgb()),
            OutputColor::ExtendedSrgbLinear => 5,
            OutputColor::ExtendedSrgb => 6,
            OutputColor::ExtendedDisplayP3 => 7,
            OutputColor::Bt2100Pq => 8,
            OutputColor::Bt2100Hlg => 9,
        };
        let alpha: u32 = match output.alpha {
            OutputAlpha::Opaque => 0,
            OutputAlpha::Premultiplied => 1,
            OutputAlpha::Straight => 2,
        };
        // The effective headroom: the display's, clamped to what this
        // destination's transfer encodes (#98).
        let ceiling = match output.color {
            OutputColor::Srgb | OutputColor::DisplayP3 => 1.0,
            OutputColor::Bt2100Pq => PQ_HEADROOM,
            OutputColor::Bt2100Hlg => HLG_HEADROOM,
            OutputColor::LinearDisplayP3
            | OutputColor::ExtendedSrgbLinear
            | OutputColor::ExtendedSrgb
            | OutputColor::ExtendedDisplayP3 => f32::MAX,
        };
        let headroom = output.headroom.max(0.0).min(ceiling);
        let mut bytes = [0; 16];
        bytes[0..4].copy_from_slice(&encode.to_ne_bytes());
        bytes[4..8].copy_from_slice(&alpha.to_ne_bytes());
        bytes[8..12].copy_from_slice(&headroom.to_bits().to_ne_bytes());
        bytes
    }

    /// Records the blit of `source` into `output.texture` into `encoder`,
    /// reading `uniform`, whose contents the caller wrote with
    /// [`Self::uniform_bytes`] for this output.
    ///
    /// # Panics
    /// If the textures violate wgpu's attachment and sampling requirements.
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        output: TextureOutput<'_>,
        uniform: &wgpu::Buffer,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let target = output.texture;
        let format = target.format();
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("present"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: timestamps,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(self.pipeline(device, format));
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Color space of a host-owned presentation texture.
#[derive(Clone, Copy, Debug)]
pub enum OutputColor {
    /// sRGB primaries and transfer; sRGB texture formats encode in hardware.
    Srgb,
    /// Display P3 primaries with the sRGB transfer — the SDR wide-gamut
    /// surface colour space. sRGB texture formats encode in hardware.
    DisplayP3,
    /// Extended linear Display P3, preserving HDR values in float targets.
    LinearDisplayP3,
    /// Extended linear sRGB (scRGB): sRGB primaries, linear transfer,
    /// negative and above-one components carried.
    ExtendedSrgbLinear,
    /// Extended-range sRGB: the sRGB transfer continued sign-symmetrically
    /// beyond `[0, 1]` (`ExtendedSrgb` surface colour space).
    ExtendedSrgb,
    /// Extended-range Display P3: the extended transfer on the working
    /// primaries (`ExtendedDisplayP3` surface colour space).
    ExtendedDisplayP3,
    /// BT.2020 primaries with the PQ transfer; SDR white calibrates to
    /// [`REFERENCE_WHITE_NITS`] of the 10 000-nit signal.
    Bt2100Pq,
    /// BT.2020 primaries with the HLG transfer; SDR white calibrates to
    /// [`REFERENCE_WHITE_NITS`] of the 1000-nit nominal peak under the
    /// BT.2100 reference OOTF (system gamma 1.2).
    Bt2100Hlg,
}

/// Alpha convention of a host-owned presentation texture.
#[derive(Clone, Copy, Debug)]
pub enum OutputAlpha {
    /// The destination is opaque.
    Opaque,
    /// Channels are multiplied by alpha in the destination encoding. For
    /// sRGB this multiplication follows transfer encoding, including when the
    /// texture format applies the transfer in hardware.
    Premultiplied,
    /// Channels are independent of alpha.
    Straight,
}

/// A host-owned texture and its presentation conventions.
#[derive(Clone, Copy, Debug)]
pub struct TextureOutput<'a> {
    /// Attachment on the same device as the source.
    pub texture: &'a wgpu::Texture,
    /// Destination color encoding.
    pub color: OutputColor,
    /// Destination alpha convention.
    pub alpha: OutputAlpha,
    /// The display's HDR headroom the destination reaches: SDR content is
    /// `1.0`. Values above `1` roll off smoothly towards it (#97); a headroom
    /// change applies to the next call, not to the content. The tone map's
    /// target never exceeds what the destination carries (#98).
    pub headroom: f32,
}

#[cfg(test)]
mod tests {
    use wgpu::{
        Backend, CompositeAlphaMode, SurfaceCapabilities, SurfaceColorSpace as Cs,
        SurfaceColorSpaces as Spaces, SurfaceFormatCapabilities, TextureFormat as F, TextureUsages,
    };

    use super::{
        DestinationPrimaries, OutputColor, OutputRequest, SelectionReason, TransferEncoding,
        select_output,
    };
    use crate::DisplaySync;

    /// A display-synchronized request.
    const fn request(transparent: bool, color_space: Option<Cs>) -> OutputRequest {
        OutputRequest {
            transparent,
            color_space,
            sync: DisplaySync::Synchronized,
        }
    }

    /// A synthetic surface: `fc` is the (format, colour spaces) list, in
    /// surface preference order; `auto` is what `formats` reports (the
    /// Auto-usable subset, in the same order).
    fn surface(
        fc: &[(F, Spaces)],
        auto: &[F],
        alpha_modes: Vec<CompositeAlphaMode>,
    ) -> SurfaceCapabilities {
        SurfaceCapabilities {
            formats: auto.to_vec(),
            format_capabilities: fc
                .iter()
                .map(|&(format, color_spaces)| SurfaceFormatCapabilities {
                    format,
                    color_spaces,
                })
                .collect(),
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes,
            usages: TextureUsages::RENDER_ATTACHMENT,
        }
    }

    fn opaque() -> Vec<CompositeAlphaMode> {
        vec![CompositeAlphaMode::Opaque]
    }

    #[test]
    fn metal_prefers_rgba16f_extended_display_p3() {
        let caps = surface(
            &[
                (F::Bgra8UnormSrgb, Spaces::SRGB),
                (
                    F::Rgba16Float,
                    Spaces::EXTENDED_DISPLAY_P3 | Spaces::EXTENDED_SRGB_LINEAR,
                ),
            ],
            &[F::Bgra8UnormSrgb, F::Rgba16Float],
            opaque(),
        );
        let sel = select_output(&caps, Backend::Metal, request(false, None)).unwrap();
        assert_eq!(sel.format, F::Rgba16Float);
        assert_eq!(sel.color_space, Cs::ExtendedDisplayP3);
        assert_eq!(sel.primaries, DestinationPrimaries::DisplayP3);
        assert_eq!(sel.transfer, TransferEncoding::ExtendedSrgb);
        assert_eq!(sel.reason, SelectionReason::PreferredHdr);
        assert_eq!(sel.tone_map_ceiling.to_bits(), f32::MAX.to_bits());
    }

    #[test]
    fn selection_pairs_formats_with_their_advertised_spaces_only() {
        // Rgba16Float carries only extended spaces; Bgra8UnormSrgb only
        // sRGB. A Cartesian product of the two lists would fabricate
        // Rgba16Float+sRGB or Bgra8+P3 pairs the surface never advertised.
        let caps = surface(
            &[
                (F::Bgra8UnormSrgb, Spaces::SRGB),
                (F::Rgba16Float, Spaces::EXTENDED_SRGB_LINEAR),
            ],
            &[F::Bgra8UnormSrgb],
            opaque(),
        );
        let sel = select_output(&caps, Backend::Metal, request(false, None)).unwrap();
        assert_eq!(
            (sel.format, sel.color_space),
            (F::Rgba16Float, Cs::ExtendedSrgbLinear)
        );
    }

    #[test]
    fn formats_absent_from_formats_reach_their_explicit_colour_space() {
        // wgpu excludes explicit-opt-in formats from `formats`; a format
        // advertised only under a non-Auto space is still selectable.
        let caps = surface(
            &[
                (F::Bgra8UnormSrgb, Spaces::SRGB),
                (F::Rgba16Float, Spaces::BT2100_PQ),
            ],
            &[F::Bgra8UnormSrgb],
            opaque(),
        );
        let sel = select_output(&caps, Backend::Vulkan, request(false, None)).unwrap();
        assert_eq!(
            (sel.format, sel.color_space),
            (F::Rgba16Float, Cs::Bt2100Pq)
        );
        assert_eq!(sel.reason, SelectionReason::PreferredHdr);
        assert!((sel.tone_map_ceiling - 10000.0 / 203.0).abs() < 1e-3);
    }

    #[test]
    fn vulkan_prefers_extended_linear_over_pq() {
        let caps = surface(
            &[
                (F::Bgra8UnormSrgb, Spaces::SRGB),
                (
                    F::Rgba16Float,
                    Spaces::EXTENDED_SRGB_LINEAR | Spaces::BT2100_PQ,
                ),
                (F::Rgb10a2Unorm, Spaces::BT2100_PQ),
            ],
            &[F::Bgra8UnormSrgb, F::Rgba16Float],
            opaque(),
        );
        let sel = select_output(&caps, Backend::Vulkan, request(false, None)).unwrap();
        assert_eq!(
            (sel.format, sel.color_space),
            (F::Rgba16Float, Cs::ExtendedSrgbLinear)
        );
    }

    #[test]
    fn vulkan_ten_bit_pq_wins_over_software_hdr() {
        let caps = surface(
            &[
                (F::Bgra8UnormSrgb, Spaces::SRGB),
                (F::Rgba16Float, Spaces::BT2100_PQ),
                (F::Rgb10a2Unorm, Spaces::BT2100_PQ),
            ],
            &[F::Bgra8UnormSrgb, F::Rgba16Float, F::Rgb10a2Unorm],
            opaque(),
        );
        let sel = select_output(&caps, Backend::Dx12, request(false, None)).unwrap();
        assert_eq!(
            (sel.format, sel.color_space),
            (F::Rgb10a2Unorm, Cs::Bt2100Pq)
        );
        assert_eq!(sel.primaries, DestinationPrimaries::Bt2100);
        assert_eq!(sel.transfer, TransferEncoding::Pq);
    }

    #[test]
    fn a_p3_only_surface_selects_wide_gamut_sdr() {
        let caps = surface(
            &[(F::Bgra8Unorm, Spaces::DISPLAY_P3)],
            &[F::Bgra8Unorm],
            opaque(),
        );
        let sel = select_output(&caps, Backend::Metal, request(false, None)).unwrap();
        assert_eq!(sel.color_space, Cs::DisplayP3);
        assert_eq!(sel.reason, SelectionReason::WideGamutSdr);
        assert_eq!(sel.primaries, DestinationPrimaries::DisplayP3);
        assert_eq!(sel.transfer, TransferEncoding::Srgb);
        assert_eq!(sel.tone_map_ceiling.to_bits(), 1.0f32.to_bits());
    }

    #[test]
    fn an_srgb_only_surface_reports_sdr_explicitly() {
        let caps = surface(
            &[(F::Bgra8UnormSrgb, Spaces::SRGB)],
            &[F::Bgra8UnormSrgb],
            opaque(),
        );
        for backend in [Backend::Metal, Backend::Vulkan, Backend::Dx12] {
            let sel = select_output(&caps, backend, request(false, None)).unwrap();
            assert_eq!(sel.color_space, Cs::Srgb);
            assert_eq!(sel.reason, SelectionReason::Sdr, "{backend:?}");
        }
    }

    #[test]
    fn a_surface_without_explicit_spaces_uses_the_legacy_pair() {
        // A backend that reports formats but no format_capabilities:
        // the historical formats.first() + Auto configuration.
        let caps = surface(&[], &[F::Bgra8UnormSrgb], opaque());
        let sel = select_output(&caps, Backend::Metal, request(false, None)).unwrap();
        assert_eq!((sel.format, sel.color_space), (F::Bgra8UnormSrgb, Cs::Auto));
        assert_eq!(sel.reason, SelectionReason::Sdr);
    }

    #[test]
    fn an_empty_surface_is_unsupported() {
        let caps = surface(&[], &[], opaque());
        assert!(matches!(
            select_output(&caps, Backend::Metal, request(false, None)),
            Err(cherenkov::SurfaceError::UnsupportedTarget(_))
        ));
    }

    #[test]
    fn a_required_colour_space_is_honoured() {
        let caps = surface(
            &[
                (F::Bgra8UnormSrgb, Spaces::SRGB),
                (
                    F::Rgba16Float,
                    Spaces::EXTENDED_DISPLAY_P3 | Spaces::BT2100_HLG,
                ),
            ],
            &[F::Bgra8UnormSrgb, F::Rgba16Float],
            opaque(),
        );
        let sel =
            select_output(&caps, Backend::Metal, request(false, Some(Cs::Bt2100Hlg))).unwrap();
        assert_eq!(
            (sel.format, sel.color_space),
            (F::Rgba16Float, Cs::Bt2100Hlg)
        );
        assert_eq!(sel.reason, SelectionReason::Required);
        assert_eq!(sel.transfer, TransferEncoding::Hlg);
        assert!((sel.tone_map_ceiling - 1000.0 / 203.0).abs() < 1e-4);
    }

    #[test]
    fn an_unadvertised_required_space_is_unsupported() {
        let caps = surface(
            &[(F::Bgra8UnormSrgb, Spaces::SRGB)],
            &[F::Bgra8UnormSrgb],
            opaque(),
        );
        for space in [Cs::Bt2100Pq, Cs::Bt2100Hlg, Cs::ExtendedDisplayP3] {
            assert!(
                matches!(
                    select_output(&caps, Backend::Metal, request(false, Some(space))),
                    Err(cherenkov::SurfaceError::UnsupportedTarget(_))
                ),
                "{space:?} must not be silently substituted"
            );
        }
    }

    #[test]
    fn a_required_space_falls_back_to_the_surfaces_format_order() {
        // The required space advertised on formats outside the preferred
        // order still resolves — first advertised format for that space.
        let caps = surface(
            &[
                (F::Bgra8Unorm, Spaces::DISPLAY_P3),
                (F::Rgba8Unorm, Spaces::DISPLAY_P3),
            ],
            &[F::Bgra8Unorm],
            opaque(),
        );
        let sel =
            select_output(&caps, Backend::Metal, request(false, Some(Cs::DisplayP3))).unwrap();
        assert_eq!(sel.format, F::Bgra8Unorm);
    }

    #[test]
    fn effective_headroom_clamps_to_the_destinations_range() {
        let caps = surface(
            &[
                (F::Bgra8UnormSrgb, Spaces::SRGB),
                (F::Rgba16Float, Spaces::EXTENDED_DISPLAY_P3),
            ],
            &[F::Bgra8UnormSrgb, F::Rgba16Float],
            opaque(),
        );
        let sel = select_output(&caps, Backend::Metal, request(false, None)).unwrap();
        assert_eq!(sel.effective_headroom(4.0).to_bits(), 4.0f32.to_bits());
        assert_eq!(sel.effective_headroom(0.0).to_bits(), 0.0f32.to_bits());

        let sdr_caps = surface(
            &[(F::Bgra8UnormSrgb, Spaces::SRGB)],
            &[F::Bgra8UnormSrgb],
            opaque(),
        );
        let sdr = select_output(&sdr_caps, Backend::Metal, request(false, None)).unwrap();
        assert_eq!(sdr.effective_headroom(8.0).to_bits(), 1.0f32.to_bits());

        let pq_caps = surface(
            &[
                (F::Bgra8UnormSrgb, Spaces::SRGB),
                (F::Rgba16Float, Spaces::BT2100_PQ),
            ],
            &[F::Bgra8UnormSrgb, F::Rgba16Float],
            opaque(),
        );
        let pq = select_output(&pq_caps, Backend::Vulkan, request(false, None)).unwrap();
        assert!((pq.effective_headroom(1e6) - 10000.0 / 203.0).abs() < 1e-3);
        assert_eq!(pq.effective_headroom(2.0).to_bits(), 2.0f32.to_bits());
    }

    #[test]
    fn headroom_is_unknown_until_the_probe_reports() {
        // `reported_headroom` stays None out of select_output — the live
        // value arrives through the DisplayProbe or a non-Metal
        // display_hdr_info read; unknown is never guessed as SDR.
        let caps = surface(
            &[(F::Rgba16Float, Spaces::EXTENDED_DISPLAY_P3)],
            &[F::Rgba16Float],
            opaque(),
        );
        let sel = select_output(&caps, Backend::Metal, request(false, None)).unwrap();
        assert!(sel.reported_headroom.is_none());
    }

    #[test]
    fn a_transparent_window_needs_a_transparency_alpha_mode() {
        let premul = surface(
            &[
                (F::Bgra8UnormSrgb, Spaces::SRGB),
                (F::Rgba16Float, Spaces::EXTENDED_DISPLAY_P3),
            ],
            &[F::Bgra8UnormSrgb, F::Rgba16Float],
            vec![
                CompositeAlphaMode::Opaque,
                CompositeAlphaMode::PreMultiplied,
            ],
        );
        let sel = select_output(&premul, Backend::Metal, request(true, None)).unwrap();
        assert_eq!(sel.alpha_mode, CompositeAlphaMode::PreMultiplied);

        let postmul = surface(
            &[(F::Bgra8UnormSrgb, Spaces::SRGB)],
            &[F::Bgra8UnormSrgb],
            vec![
                CompositeAlphaMode::Opaque,
                CompositeAlphaMode::PostMultiplied,
            ],
        );
        let sel = select_output(&postmul, Backend::Metal, request(true, None)).unwrap();
        assert_eq!(sel.alpha_mode, CompositeAlphaMode::PostMultiplied);

        let opaque_only = surface(
            &[(F::Bgra8UnormSrgb, Spaces::SRGB)],
            &[F::Bgra8UnormSrgb],
            vec![CompositeAlphaMode::Opaque],
        );
        assert!(matches!(
            select_output(&opaque_only, Backend::Metal, request(true, None)),
            Err(cherenkov::SurfaceError::UnsupportedTarget(_))
        ));
    }

    #[test]
    fn an_opaque_window_prefers_opaque_alpha() {
        let caps = surface(
            &[(F::Bgra8UnormSrgb, Spaces::SRGB)],
            &[F::Bgra8UnormSrgb],
            vec![
                CompositeAlphaMode::PreMultiplied,
                CompositeAlphaMode::Opaque,
            ],
        );
        let sel = select_output(&caps, Backend::Metal, request(false, None)).unwrap();
        assert_eq!(sel.alpha_mode, CompositeAlphaMode::Opaque);
    }

    #[test]
    fn extended_p3_is_not_raw_linear_p3() {
        // An ExtendedDisplayP3 surface gets the signed extended transfer
        // — writing the retained linear-P3 values raw would be wrong.
        let caps = surface(
            &[(F::Rgba16Float, Spaces::EXTENDED_DISPLAY_P3)],
            &[],
            opaque(),
        );
        let sel = select_output(&caps, Backend::Metal, request(false, None)).unwrap();
        assert_eq!(sel.transfer, TransferEncoding::ExtendedSrgb);
        assert!(matches!(sel.output_color(), OutputColor::ExtendedDisplayP3));
    }

    /// An sRGB surface presenting with `modes`.
    fn presenting(modes: &[wgpu::PresentMode]) -> SurfaceCapabilities {
        SurfaceCapabilities {
            present_modes: modes.to_vec(),
            ..surface(
                &[(F::Bgra8UnormSrgb, Spaces::SRGB)],
                &[F::Bgra8UnormSrgb],
                opaque(),
            )
        }
    }

    fn sync_request(sync: DisplaySync) -> OutputRequest {
        OutputRequest {
            sync,
            ..request(false, None)
        }
    }

    #[test]
    fn display_sync_resolves_to_an_advertised_present_mode() {
        use wgpu::PresentMode as M;
        let every = presenting(&[M::Immediate, M::FifoRelaxed, M::Mailbox, M::Fifo]);
        let sync = select_output(
            &every,
            Backend::Vulkan,
            sync_request(DisplaySync::Synchronized),
        )
        .unwrap();
        assert_eq!(
            sync.present_mode,
            M::Fifo,
            "relaxed FIFO tears a late frame"
        );
        let unsync = select_output(
            &every,
            Backend::Vulkan,
            sync_request(DisplaySync::Unsynchronized),
        )
        .unwrap();
        assert_eq!(unsync.present_mode, M::Mailbox, "mailbox where offered");
        // What wgpu's Metal backend advertises on macOS.
        let mac = presenting(&[M::Fifo, M::Immediate]);
        let unsync = select_output(
            &mac,
            Backend::Metal,
            sync_request(DisplaySync::Unsynchronized),
        )
        .unwrap();
        assert_eq!(unsync.present_mode, M::Immediate, "immediate otherwise");
    }

    #[test]
    fn unsynchronized_presentation_without_mailbox_or_immediate_is_unsupported() {
        use wgpu::PresentMode as M;
        // A FIFO-only surface (Metal on iOS, WebGPU), and one whose only
        // other mode still waits for the display when frames keep up.
        for modes in [&[M::Fifo][..], &[M::Fifo, M::FifoRelaxed]] {
            assert!(
                matches!(
                    select_output(
                        &presenting(modes),
                        Backend::Metal,
                        sync_request(DisplaySync::Unsynchronized),
                    ),
                    Err(cherenkov::SurfaceError::UnsupportedTarget(_))
                ),
                "{modes:?} must not be substituted for unsynchronized presentation"
            );
        }
    }
}
