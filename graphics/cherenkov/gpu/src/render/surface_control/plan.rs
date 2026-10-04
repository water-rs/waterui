//! What one `ASurfaceTransaction` sets on one child surface control.
//!
//! This is the part of the `SurfaceControl` realization that involves no NDK
//! call: the dataspace a frame's colour contract maps to, the source crop,
//! destination rectangle and buffer transform a layer's device transform
//! and clip map to, the z-order of a stack of planes, and which properties a
//! transaction has to set given what the previous one set. Everything here
//! is exact or reports the contract as inexpressible; nothing is
//! approximated into a different contract.

use cherenkov::ShapeData;
use cherenkov::kurbo::{Affine, Rect};

use crate::interop::{FrameColor, HdrMetadata, Primaries, RgbAlpha, Transfer, YuvMatrix, YuvRange};
use crate::render::planes::{self, Level};

/// The `ADataSpace` standard field: primaries and, for YUV, the matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standard {
    /// `STANDARD_BT709`: BT.709 primaries and matrix.
    Bt709,
    /// `STANDARD_BT2020`: BT.2020 primaries and non-constant-luminance
    /// matrix.
    Bt2020,
    /// `STANDARD_DCI_P3`: P3 primaries. Android reads it with a D65 white
    /// point for RGB content (`ADATASPACE_DISPLAY_P3`).
    DciP3,
}

/// The `ADataSpace` transfer field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferFn {
    /// `TRANSFER_LINEAR`.
    Linear,
    /// `TRANSFER_SRGB`.
    Srgb,
    /// `TRANSFER_SMPTE_170M`: the BT.601/BT.709 OETF.
    Smpte170M,
    /// `TRANSFER_ST2084`: PQ.
    St2084,
    /// `TRANSFER_HLG`.
    Hlg,
}

/// The `ADataSpace` range field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    /// `RANGE_FULL`.
    Full,
    /// `RANGE_LIMITED`: studio-range codes.
    Limited,
    /// `RANGE_EXTENDED`: float values beyond `[0, 1]` are meaningful.
    Extended,
}

/// A buffer dataspace, field by field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dataspace {
    /// Primaries and matrix.
    pub standard: Standard,
    /// Transfer function.
    pub transfer: TransferFn,
    /// Code range.
    pub range: Range,
}

impl Dataspace {
    /// `ADATASPACE_SRGB`: what the engine's own planes carry.
    pub const SRGB: Self = Self {
        standard: Standard::Bt709,
        transfer: TransferFn::Srgb,
        range: Range::Full,
    };
}

/// How a frame's buffer encodes its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// `Y'CbCr` codes, decoded by the matrix and range.
    Yuv,
    /// A single RGB(A) plane.
    Rgb {
        /// Whether the plane stores floats, whose values beyond `[0, 1]` the
        /// dataspace must keep.
        float: bool,
        /// How the plane's alpha composes.
        alpha: RgbAlpha,
    },
}

/// Why a layer's state cannot be expressed on a system compositor plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Inexpressible {
    /// The frame's primaries and `Y'CbCr` matrix are not one `ADataSpace`
    /// standard.
    #[error("no ADataSpace standard carries {primaries:?} primaries with the {matrix:?} matrix")]
    Standard {
        /// The frame's primaries.
        primaries: Primaries,
        /// The frame's matrix.
        matrix: YuvMatrix,
    },
    /// An absolute transfer's reference white is not the BT.2408 level
    /// (203 nits) the system compositor maps to SDR white.
    #[error("a PQ or HLG frame composes on a plane only at the 203-nit reference white")]
    ReferenceWhite,
    /// Straight alpha: a plane blends premultiplied.
    #[error("a plane blends premultiplied; straight alpha has no plane equivalent")]
    StraightAlpha,
    /// The transform shears or rotates by other than a multiple of 90°.
    #[error("the layer's device transform is not axis-aligned")]
    Transform,
    /// The transform collapses the frame, or is not finite.
    #[error("the layer's device transform is degenerate")]
    Degenerate,
    /// A clip on the layer's path is not a rectangle.
    #[error("a clip on the layer's path is not a rectangle")]
    Clip,
}

/// The reference white the system compositor maps PQ and HLG signals on:
/// the BT.2408 HDR reference level.
const SYSTEM_REFERENCE_WHITE: f32 = 203.0;

/// The dataspace a frame's colour contract maps to.
///
/// Chroma siting is not part of a dataspace: a buffer's own metadata carries
/// it, and the import already requires the frame's declared siting to match
/// what the buffer reports.
///
/// # Errors
/// [`Inexpressible`] when no dataspace carries the contract exactly.
pub fn dataspace(color: &FrameColor, encoding: Encoding) -> Result<Dataspace, Inexpressible> {
    let (standard, range) = match encoding {
        Encoding::Yuv => {
            let standard = match (color.primaries, color.matrix) {
                (Primaries::Bt709, YuvMatrix::Bt709) => Standard::Bt709,
                (Primaries::Bt2020, YuvMatrix::Bt2020) => Standard::Bt2020,
                (primaries, matrix) => return Err(Inexpressible::Standard { primaries, matrix }),
            };
            let range = match color.range {
                YuvRange::Video => Range::Limited,
                YuvRange::Full => Range::Full,
            };
            (standard, range)
        }
        Encoding::Rgb { float, .. } => {
            let standard = match color.primaries {
                Primaries::Bt709 => Standard::Bt709,
                Primaries::DisplayP3 => Standard::DciP3,
                Primaries::Bt2020 => Standard::Bt2020,
            };
            (standard, if float { Range::Extended } else { Range::Full })
        }
    };
    // Per the NDK ADataSpace documentation, each transfer function is a
    // pixel transform applied by the compositor: `Smpte170M` is
    // "transfer characteristic SMPTE 170M" — the same OETF family the
    // engine approximates with BT.1886 γ2.4 — `St2084` the SMPTE ST 2084
    // perceptual quantizer, `Hlg` hybrid log-gamma, `Srgb` the sRGB
    // transfer function and `Linear` a no-op. Which of these decodes
    // SurfaceFlinger performs exactly matching the engine's decode is a
    // platform claim that needs device evidence before `shows` can
    // refuse one; none is excluded yet.
    let transfer = match color.transfer {
        Transfer::Linear => TransferFn::Linear,
        Transfer::Srgb => TransferFn::Srgb,
        Transfer::Bt709 => TransferFn::Smpte170M,
        Transfer::Pq => TransferFn::St2084,
        Transfer::Hlg => TransferFn::Hlg,
    };
    let other_white = color.reference_white != SYSTEM_REFERENCE_WHITE;
    if matches!(transfer, TransferFn::St2084 | TransferFn::Hlg) && other_white {
        return Err(Inexpressible::ReferenceWhite);
    }
    Ok(Dataspace {
        standard,
        transfer,
        range,
    })
}

/// Whether a frame's buffer is opaque on its plane.
///
/// # Errors
/// [`Inexpressible::StraightAlpha`] for straight alpha, which a plane cannot
/// blend.
pub const fn opaque(encoding: Encoding) -> Result<bool, Inexpressible> {
    match encoding {
        Encoding::Yuv
        | Encoding::Rgb {
            alpha: RgbAlpha::Opaque,
            ..
        } => Ok(true),
        Encoding::Rgb {
            alpha: RgbAlpha::Premultiplied,
            ..
        } => Ok(false),
        Encoding::Rgb {
            alpha: RgbAlpha::Straight,
            ..
        } => Err(Inexpressible::StraightAlpha),
    }
}

/// An integer rectangle, edges inclusive-exclusive, as `ARect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IRect {
    /// Left edge.
    pub left: i32,
    /// Top edge.
    pub top: i32,
    /// Right edge (exclusive).
    pub right: i32,
    /// Bottom edge (exclusive).
    pub bottom: i32,
}

impl IRect {
    /// `rect` with each edge rounded to the nearest integer.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "edges are bounded by surface and frame sizes, which fit i32"
    )]
    const fn round(rect: Rect) -> Self {
        Self {
            left: rect.x0.round() as i32,
            top: rect.y0.round() as i32,
            right: rect.x1.round() as i32,
            bottom: rect.y1.round() as i32,
        }
    }

    const fn is_empty(self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }
}

/// A buffer transform, as the `ANATIVEWINDOW_TRANSFORM_*` bits: the mirrors
/// apply first, then the clockwise quarter turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BufferTransform {
    /// `ANATIVEWINDOW_TRANSFORM_MIRROR_HORIZONTAL`.
    pub mirror_x: bool,
    /// `ANATIVEWINDOW_TRANSFORM_MIRROR_VERTICAL`.
    pub mirror_y: bool,
    /// `ANATIVEWINDOW_TRANSFORM_ROTATE_90`, clockwise.
    pub rotate_90: bool,
}

/// Where a plane's buffer lands: the crop in buffer pixels, the rectangle in
/// the parent's space it scales into, and the transform between them —
/// `ASurfaceTransaction_setGeometry`'s three arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    /// The source crop, in buffer pixels.
    pub source: IRect,
    /// The destination, in the parent's (the engine surface's device) space.
    pub destination: IRect,
    /// The buffer transform applied after the crop.
    pub transform: BufferTransform,
}

/// A plane's placement: shown with a geometry, or hidden because nothing
/// of it is visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Shown.
    Visible(Geometry),
    /// Clipped away entirely.
    Hidden,
}

/// The most layers promoted on one Android surface.
///
/// A promoted layer adds two layers to the system's composition: its own
/// plane and the engine part painted above it. One promotion keeps a
/// surface at three layers, which a hardware composer scans out alongside
/// the system bars without falling back to client composition; it covers
/// the case planes exist for, a video under its controls.
pub const BUDGET: usize = 1;

/// Whether a child surface control carries a layer's local `transform`
/// exactly: the product of such transforms is again a [`buffer_transform`].
#[must_use]
pub fn expresses_transform(transform: Affine) -> bool {
    buffer_transform(transform).is_ok()
}

/// Whether a child surface control carries `clip` exactly: a crop, which is
/// a rectangle.
#[must_use]
pub const fn expresses_clip(clip: &ShapeData) -> bool {
    matches!(clip, ShapeData::Rect(_))
}

/// The placement on `surface` of a layer [`planes::plan`] promoted.
///
/// # Errors
/// [`Inexpressible`] when a level on its path is not expressible, which the
/// plan's [`expresses_transform`] and [`expresses_clip`] checks exclude.
pub fn promoted(
    promoted: &planes::Placement,
    surface: (u32, u32),
) -> Result<Placement, Inexpressible> {
    placement(
        promoted.content_to_device(),
        promoted.size,
        device_clip(&promoted.path)?,
        surface,
    )
}

/// The buffer transform a layer's device `transform` maps to: an
/// axis-aligned scale and translation, optionally mirrored and turned by a
/// quarter. A system layer carries exactly these.
///
/// # Errors
/// [`Inexpressible::Transform`] when the transform shears or turns by other
/// than a multiple of 90°; [`Inexpressible::Degenerate`] when it is not
/// finite or collapses an axis.
pub fn buffer_transform(transform: Affine) -> Result<BufferTransform, Inexpressible> {
    let coeffs = transform.as_coeffs();
    if !coeffs.iter().all(|v| v.is_finite()) {
        return Err(Inexpressible::Degenerate);
    }
    // `x' = sx·x + kx·y + tx`, `y' = ky·x + sy·y + ty`.
    let [sx, ky, kx, sy, _, _] = coeffs;
    let scale = sx.abs().max(ky.abs()).max(kx.abs()).max(sy.abs());
    let tiny = |v: f64| v.abs() <= scale * 1e-6;
    if tiny(ky) && tiny(kx) {
        if tiny(sx) || tiny(sy) {
            return Err(Inexpressible::Degenerate);
        }
        Ok(BufferTransform {
            mirror_x: sx < 0.0,
            mirror_y: sy < 0.0,
            rotate_90: false,
        })
    } else if tiny(sx) && tiny(sy) {
        if tiny(ky) || tiny(kx) {
            return Err(Inexpressible::Degenerate);
        }
        // `x' = kx·y + tx`, `y' = ky·x + ty`: a quarter turn clockwise maps the
        // mirrored `(x₁, y₁)` to `(h − y₁, x₁)`, so device x grows with
        // buffer y exactly when the buffer is mirrored vertically, and
        // device y grows with buffer x exactly when it is not mirrored
        // horizontally.
        Ok(BufferTransform {
            mirror_x: ky < 0.0,
            mirror_y: kx > 0.0,
            rotate_90: true,
        })
    } else {
        Err(Inexpressible::Transform)
    }
}

/// The device-space clip of a layer at the end of `path`, root first: the
/// intersection of every level's clip, each mapped from the level's own
/// space. `None` when no level clips.
///
/// # Errors
/// [`Inexpressible::Clip`] when a clip is not a rectangle, and the
/// [`buffer_transform`] errors when a clipping level's space is not
/// axis-aligned, so its rectangle does not map to a device rectangle.
pub fn device_clip(path: &[Level]) -> Result<Option<Rect>, Inexpressible> {
    let mut space = Affine::IDENTITY;
    let mut clip: Option<Rect> = None;
    for level in path {
        let own = space * level.transform;
        if let Some(shape) = &level.clip {
            let ShapeData::Rect(rect) = shape else {
                return Err(Inexpressible::Clip);
            };
            buffer_transform(own)?;
            let device = own.transform_rect_bbox(*rect);
            clip = Some(clip.map_or(device, |outer| outer.intersect(device)));
        }
        space *= level.content_transform();
    }
    Ok(clip)
}

/// The placement of a `frame`-sized buffer drawn at layer-local
/// `(0, 0)..frame` under the device `transform`, clipped to the device-space
/// `clip` and to the `surface`.
///
/// Planes position on the device pixel grid: the destination and the crop
/// round to whole pixels, which moves an edge by at most half a pixel —
/// within the perceptual tolerance promotion is verified against, and what
/// a hardware overlay's integer display frame does anyway.
///
/// # Errors
/// The [`buffer_transform`] errors.
pub fn placement(
    transform: Affine,
    frame: (u32, u32),
    clip: Option<Rect>,
    surface: (u32, u32),
) -> Result<Placement, Inexpressible> {
    let buffer_transform = buffer_transform(transform)?;
    let local = Rect::new(0.0, 0.0, f64::from(frame.0), f64::from(frame.1));
    let bounds = Rect::new(0.0, 0.0, f64::from(surface.0), f64::from(surface.1));
    let mut visible = transform.transform_rect_bbox(local).intersect(bounds);
    if let Some(clip) = clip {
        visible = visible.intersect(clip);
    }
    if visible.width() <= 0.0 || visible.height() <= 0.0 {
        return Ok(Placement::Hidden);
    }
    let source = IRect::round(
        transform
            .inverse()
            .transform_rect_bbox(visible)
            .intersect(local),
    );
    let destination = IRect::round(visible);
    if source.is_empty() || destination.is_empty() {
        return Ok(Placement::Hidden);
    }
    Ok(Placement::Visible(Geometry {
        source,
        destination,
        transform: buffer_transform,
    }))
}

/// One slot of a surface's stack of system layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Engine part `n`.
    Part(usize),
    /// Promoted plane `n`.
    Plane(usize),
}

/// The bottom-to-top order of `parts` engine parts and `planes` promoted
/// planes: plane `n` sits above part `n`, painted before it, and below part
/// `n + 1`, painted after it.
pub fn stack_order(parts: usize, planes: usize) -> impl Iterator<Item = Slot> {
    (0..parts.max(planes)).flat_map(move |n| {
        (n < parts)
            .then_some(Slot::Part(n))
            .into_iter()
            .chain((n < planes).then_some(Slot::Plane(n)))
    })
}

/// The z-order of the plane at `index` in a stack listed bottom to top.
///
/// Siblings with equal z have no defined order, so every plane in a stack
/// gets its own, strictly increasing with its position.
///
/// # Panics
/// When the stack is longer than `i32::MAX`.
#[must_use]
pub fn z_order(index: usize) -> i32 {
    i32::try_from(index).expect("a plane stack fits i32")
}

/// Every property a transaction sets on one child surface control, except
/// its buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Properties {
    /// The z-order among the engine surface's planes.
    pub z: i32,
    /// Shown with a geometry, or hidden.
    pub placement: Placement,
    /// The plane's opacity, premultiplied into its blend.
    pub alpha: f32,
    /// Whether every pixel of the buffer is opaque.
    pub opaque: bool,
    /// The buffer's dataspace.
    pub dataspace: Dataspace,
    /// Static HDR metadata for the system's tone mapping.
    pub hdr: HdrMetadata,
}

/// One property a transaction sets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    /// `setZOrder`.
    Z(i32),
    /// `setVisibility`.
    Visible(bool),
    /// `setGeometry`.
    Geometry(Geometry),
    /// `setBufferAlpha`.
    Alpha(f32),
    /// `setBufferTransparency`: opaque or translucent.
    Opaque(bool),
    /// `setBufferDataSpace`.
    Dataspace(Dataspace),
    /// `setHdrMetadata_smpte2086` and `setHdrMetadata_cta861_3`.
    Hdr(HdrMetadata),
}

/// Appends to `ops` what a transaction must set to move a surface control
/// from `prev` (`None` for one never set) to `next`.
pub fn diff(prev: Option<&Properties>, next: &Properties, ops: &mut Vec<Op>) {
    let changed = |same: fn(&Properties, &Properties) -> bool| prev.is_none_or(|p| !same(p, next));
    if changed(|p, n| p.z == n.z) {
        ops.push(Op::Z(next.z));
    }
    let was = prev.map(|p| p.placement);
    match next.placement {
        Placement::Hidden => {
            if was != Some(Placement::Hidden) {
                ops.push(Op::Visible(false));
            }
        }
        Placement::Visible(geometry) => {
            if !matches!(was, Some(Placement::Visible(_))) {
                ops.push(Op::Visible(true));
            }
            if was != Some(next.placement) {
                ops.push(Op::Geometry(geometry));
            }
        }
    }
    #[expect(
        clippy::float_cmp,
        reason = "an unchanged opacity is the same value, not a nearby one"
    )]
    let alpha_changed = changed(|p, n| p.alpha == n.alpha);
    if alpha_changed {
        ops.push(Op::Alpha(next.alpha));
    }
    if changed(|p, n| p.opaque == n.opaque) {
        ops.push(Op::Opaque(next.opaque));
    }
    if changed(|p, n| p.dataspace == n.dataspace) {
        ops.push(Op::Dataspace(next.dataspace));
    }
    if changed(|p, n| p.hdr == n.hdr) {
        ops.push(Op::Hdr(next.hdr));
    }
}

#[cfg(test)]
mod tests {
    use cherenkov::kurbo::{RoundedRect, Vec2};
    use cherenkov::testing::LayerOp;
    use cherenkov::{LayerId, Prop, SurfaceTree};

    use super::*;
    use crate::interop::{ChromaSiting, ContentLight, MasteringDisplay};

    const YUV: Encoding = Encoding::Yuv;

    fn visible(placement: Placement) -> Geometry {
        match placement {
            Placement::Visible(geometry) => geometry,
            Placement::Hidden => panic!("expected a visible placement"),
        }
    }

    const fn rect(left: i32, top: i32, right: i32, bottom: i32) -> IRect {
        IRect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn video_dataspaces_map_field_by_field() {
        assert_eq!(
            dataspace(&FrameColor::BT709_VIDEO, YUV),
            Ok(Dataspace {
                standard: Standard::Bt709,
                transfer: TransferFn::Smpte170M,
                range: Range::Limited,
            })
        );
        assert_eq!(
            dataspace(&FrameColor::BT2020_PQ, YUV),
            Ok(Dataspace {
                standard: Standard::Bt2020,
                transfer: TransferFn::St2084,
                range: Range::Limited,
            })
        );
        let hlg_full = FrameColor {
            range: YuvRange::Full,
            ..FrameColor::bt2020_hlg(1000.0)
        };
        assert_eq!(
            dataspace(&hlg_full, YUV),
            Ok(Dataspace {
                standard: Standard::Bt2020,
                transfer: TransferFn::Hlg,
                range: Range::Full,
            })
        );
    }

    #[test]
    fn rgb_dataspaces_ignore_the_matrix_and_extend_floats() {
        let rgb8 = Encoding::Rgb {
            float: false,
            alpha: RgbAlpha::Opaque,
        };
        let rgb16f = Encoding::Rgb {
            float: true,
            alpha: RgbAlpha::Premultiplied,
        };
        assert_eq!(dataspace(&FrameColor::SRGB, rgb8), Ok(Dataspace::SRGB));
        assert_eq!(opaque(rgb8), Ok(true));
        assert_eq!(opaque(rgb16f), Ok(false));
        assert_eq!(
            dataspace(&FrameColor::LINEAR_P3, rgb16f),
            Ok(Dataspace {
                standard: Standard::DciP3,
                transfer: TransferFn::Linear,
                range: Range::Extended,
            })
        );
    }

    #[test]
    fn inexpressible_colour_contracts_are_named() {
        let bt601_on_709 = FrameColor {
            matrix: YuvMatrix::Bt601,
            ..FrameColor::BT709_VIDEO
        };
        assert_eq!(
            dataspace(&bt601_on_709, YUV),
            Err(Inexpressible::Standard {
                primaries: Primaries::Bt709,
                matrix: YuvMatrix::Bt601,
            })
        );
        let dim_pq = FrameColor {
            reference_white: 100.0,
            ..FrameColor::BT2020_PQ
        };
        assert_eq!(dataspace(&dim_pq, YUV), Err(Inexpressible::ReferenceWhite));
        // A relative transfer's reference white is documentation only.
        let srgb_100 = FrameColor {
            reference_white: 100.0,
            chroma_siting: ChromaSiting::LEFT,
            ..FrameColor::SRGB
        };
        assert!(
            dataspace(
                &srgb_100,
                Encoding::Rgb {
                    float: false,
                    alpha: RgbAlpha::Opaque
                }
            )
            .is_ok()
        );
        assert_eq!(
            opaque(Encoding::Rgb {
                float: false,
                alpha: RgbAlpha::Straight
            }),
            Err(Inexpressible::StraightAlpha)
        );
        assert_eq!(opaque(YUV), Ok(true));
    }

    #[test]
    fn scale_and_translation_place_the_whole_frame() {
        let geometry = visible(
            placement(
                Affine::translate((100.0, 50.0)) * Affine::scale(0.5),
                (1920, 1080),
                None,
                (2000, 2000),
            )
            .unwrap(),
        );
        assert_eq!(geometry.source, rect(0, 0, 1920, 1080));
        assert_eq!(geometry.destination, rect(100, 50, 1060, 590));
        assert_eq!(geometry.transform, BufferTransform::default());
    }

    #[test]
    fn a_clip_crops_source_and_destination_together() {
        // The frame is scaled 2× at (10, 20); the clip keeps device x in
        // 30..130 and y in 20..60, i.e. buffer x 10..60 and y 0..20.
        let geometry = visible(
            placement(
                Affine::translate((10.0, 20.0)) * Affine::scale(2.0),
                (100, 100),
                Some(Rect::new(30.0, 0.0, 130.0, 60.0)),
                (1000, 1000),
            )
            .unwrap(),
        );
        assert_eq!(geometry.destination, rect(30, 20, 130, 60));
        assert_eq!(geometry.source, rect(10, 0, 60, 20));
    }

    #[test]
    fn the_surface_bounds_crop_like_a_clip() {
        let geometry = visible(
            placement(
                Affine::translate((-50.0, 0.0)),
                (200, 100),
                None,
                (100, 100),
            )
            .unwrap(),
        );
        assert_eq!(geometry.destination, rect(0, 0, 100, 100));
        assert_eq!(geometry.source, rect(50, 0, 150, 100));
        assert_eq!(
            placement(
                Affine::translate((0.0, 0.0)),
                (10, 10),
                Some(Rect::new(20.0, 20.0, 30.0, 30.0)),
                (100, 100)
            ),
            Ok(Placement::Hidden)
        );
    }

    #[test]
    fn mirrors_and_quarter_turns_map_to_buffer_transforms() {
        let mirrored = visible(
            placement(
                Affine::new([-1.0, 0.0, 0.0, 1.0, 100.0, 0.0]),
                (100, 50),
                None,
                (200, 200),
            )
            .unwrap(),
        );
        assert_eq!(
            mirrored.transform,
            BufferTransform {
                mirror_x: true,
                mirror_y: false,
                rotate_90: false,
            }
        );
        assert_eq!(mirrored.destination, rect(0, 0, 100, 50));
        // A quarter turn clockwise on a y-down screen: (x, y) → (−y, x).
        let quarter = visible(
            placement(
                Affine::new([0.0, 1.0, -1.0, 0.0, 50.0, 0.0]),
                (100, 50),
                None,
                (200, 200),
            )
            .unwrap(),
        );
        assert_eq!(
            quarter.transform,
            BufferTransform {
                mirror_x: false,
                mirror_y: false,
                rotate_90: true,
            }
        );
        assert_eq!(quarter.destination, rect(0, 0, 50, 100));
        assert_eq!(quarter.source, rect(0, 0, 100, 50));
        // Three quarters: (x, y) → (y, −x) is both mirrors then a quarter.
        let three = visible(
            placement(
                Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, 100.0]),
                (100, 50),
                None,
                (200, 200),
            )
            .unwrap(),
        );
        assert_eq!(
            three.transform,
            BufferTransform {
                mirror_x: true,
                mirror_y: true,
                rotate_90: true,
            }
        );
    }

    #[test]
    fn a_quarter_turn_crops_in_buffer_space() {
        // (x, y) → (−y + 50, x): device x 0..50 is buffer y 50..0, device
        // y 0..100 is buffer x 0..100. Clipping device y to 0..40 keeps
        // buffer x 0..40 over the whole buffer height.
        let geometry = visible(
            placement(
                Affine::new([0.0, 1.0, -1.0, 0.0, 50.0, 0.0]),
                (100, 50),
                Some(Rect::new(0.0, 0.0, 200.0, 40.0)),
                (200, 200),
            )
            .unwrap(),
        );
        assert_eq!(geometry.destination, rect(0, 0, 50, 40));
        assert_eq!(geometry.source, rect(0, 0, 40, 50));
    }

    #[test]
    fn shear_rotation_and_collapse_are_inexpressible() {
        assert_eq!(
            placement(Affine::rotate(0.3), (10, 10), None, (100, 100)),
            Err(Inexpressible::Transform)
        );
        assert_eq!(
            placement(Affine::skew(0.2, 0.0), (10, 10), None, (100, 100)),
            Err(Inexpressible::Transform)
        );
        assert_eq!(
            placement(
                Affine::scale_non_uniform(1.0, 0.0),
                (10, 10),
                None,
                (100, 100)
            ),
            Err(Inexpressible::Degenerate)
        );
        assert_eq!(
            placement(
                Affine::translate((f64::NAN, 0.0)),
                (10, 10),
                None,
                (100, 100)
            ),
            Err(Inexpressible::Degenerate)
        );
    }

    /// The plan's compositor on Android, with every frame showable: what
    /// the path rules alone decide.
    struct Android;

    impl planes::Compositor for Android {
        const BUDGET: usize = BUDGET;
        fn expresses_transform(transform: Affine) -> bool {
            expresses_transform(transform)
        }
        fn expresses_clip(clip: &ShapeData) -> bool {
            expresses_clip(clip)
        }
        fn shows(_: &crate::interop::ExternalFrame) -> bool {
            true
        }
    }

    const ROOT: LayerId = LayerId::new(0);
    const PARENT: LayerId = LayerId::new(1);
    const VIDEO: LayerId = LayerId::new(2);
    const CONTROLS: LayerId = LayerId::new(3);

    const fn prop<T>(target: T) -> Prop<T> {
        Prop {
            target,
            animation: None,
        }
    }

    /// `ROOT` holding `PARENT` (holding `VIDEO`) and then `CONTROLS`.
    fn player() -> SurfaceTree {
        let mut tree = SurfaceTree::new();
        for id in [PARENT, VIDEO, CONTROLS] {
            tree.apply(LayerOp::Create(id));
        }
        for (parent, child) in [(ROOT, PARENT), (PARENT, VIDEO), (ROOT, CONTROLS)] {
            tree.apply(LayerOp::Push { parent, child });
        }
        tree
    }

    fn decide(tree: &SurfaceTree, size: (u32, u32)) -> planes::Plan {
        planes::plan::<Android>(
            tree,
            &std::iter::once((VIDEO, size.into())).collect(),
            &std::iter::once(VIDEO).collect(),
        )
    }

    /// A video scaled into a scrolled, clipped parent lands where the
    /// engine draws it: the parent's clip in its own space, the scroll
    /// applied to its content, the scale to the video's.
    #[test]
    fn a_promoted_layer_is_placed_through_its_path() {
        let mut tree = player();
        tree.apply(LayerOp::Transform(
            PARENT,
            prop(Affine::translate((40.0, 30.0))),
        ));
        tree.apply(LayerOp::Clip(
            PARENT,
            Some(ShapeData::Rect(Rect::new(0.0, 0.0, 200.0, 100.0))),
        ));
        tree.apply(LayerOp::ScrollOffset(PARENT, prop(Vec2::new(0.0, 20.0))));
        tree.apply(LayerOp::Transform(VIDEO, prop(Affine::scale(0.5))));
        let plan = decide(&tree, (640, 360));
        assert!(plan.rejected.is_empty(), "{:?}", plan.rejected);
        let geometry = visible(promoted(&plan.planes[0], (1000, 1000)).unwrap());
        // Content space to device: translate(40, 30 - 20) * scale(0.5), so
        // the video covers device (40, 10)..(360, 190); the parent clips
        // device (40, 30)..(240, 130).
        assert_eq!(geometry.destination, rect(40, 30, 240, 130));
        assert_eq!(geometry.source, rect(0, 40, 400, 240));
        assert!(plan.trailing, "the controls stay in a part above");
    }

    /// Nested clips intersect, each in its own level's space.
    #[test]
    fn nested_clips_intersect_in_device_space() {
        let level = |transform, clip, scroll| Level {
            layer: ROOT,
            transform,
            clip: Some(ShapeData::Rect(clip)),
            scroll,
        };
        let path = [
            level(
                Affine::IDENTITY,
                Rect::new(0.0, 0.0, 100.0, 100.0),
                Vec2::new(0.0, 30.0),
            ),
            level(
                Affine::translate((10.0, 10.0)) * Affine::scale(2.0),
                Rect::new(0.0, 0.0, 20.0, 20.0),
                Vec2::ZERO,
            ),
        ];
        // The inner clip is (10, -20)..(50, 20) on the device, the outer
        // (0, 0)..(100, 100).
        assert_eq!(
            device_clip(&path),
            Ok(Some(Rect::new(10.0, 0.0, 50.0, 20.0)))
        );
        assert_eq!(device_clip(&path[..0]), Ok(None));
        let rounded = Level {
            clip: Some(ShapeData::RoundedRect(RoundedRect::new(
                0.0, 0.0, 10.0, 10.0, 2.0,
            ))),
            ..path[0].clone()
        };
        assert_eq!(device_clip(&[rounded]), Err(Inexpressible::Clip));
    }

    /// The plan keeps a video in the engine when its path rotates or clips
    /// with a shape, and promotes it through mirrors and quarter turns.
    #[test]
    fn the_path_rules_follow_what_a_surface_control_carries() {
        let mut tree = player();
        tree.apply(LayerOp::Transform(PARENT, prop(Affine::rotate(0.3))));
        assert_eq!(
            decide(&tree, (16, 16)).rejected,
            [(VIDEO, planes::Ineligible::Transform(PARENT))]
        );
        let mut tree = player();
        tree.apply(LayerOp::Clip(
            VIDEO,
            Some(ShapeData::RoundedRect(RoundedRect::new(
                0.0, 0.0, 10.0, 10.0, 2.0,
            ))),
        ));
        assert_eq!(
            decide(&tree, (16, 16)).rejected,
            [(VIDEO, planes::Ineligible::Clip(VIDEO))]
        );
        let mut tree = player();
        tree.apply(LayerOp::Transform(
            PARENT,
            prop(Affine::new([0.0, 1.0, -1.0, 0.0, 50.0, 0.0])),
        ));
        tree.apply(LayerOp::Transform(
            VIDEO,
            prop(Affine::new([-1.0, 0.0, 0.0, 1.0, 16.0, 0.0])),
        ));
        assert_eq!(decide(&tree, (16, 16)).planes.len(), 1);
    }

    /// Each plane sits between the part painted before it and the part
    /// painted after it, and nothing exists above a topmost plane.
    #[test]
    fn planes_interleave_with_the_parts_around_them() {
        use Slot::{Part, Plane};
        assert_eq!(
            stack_order(3, 2).collect::<Vec<_>>(),
            [Part(0), Plane(0), Part(1), Plane(1), Part(2)]
        );
        assert_eq!(stack_order(1, 1).collect::<Vec<_>>(), [Part(0), Plane(0)]);
        assert_eq!(stack_order(1, 0).collect::<Vec<_>>(), [Part(0)]);
    }

    #[test]
    fn a_stack_orders_strictly_bottom_to_top() {
        let z: Vec<i32> = (0..5).map(z_order).collect();
        assert!(z.windows(2).all(|w| w[0] < w[1]), "{z:?}");
        assert_eq!(z[0], 0, "the stack starts above the parent's own content");
    }

    fn props() -> Properties {
        Properties {
            z: 1,
            placement: Placement::Visible(Geometry {
                source: rect(0, 0, 10, 10),
                destination: rect(0, 0, 20, 20),
                transform: BufferTransform::default(),
            }),
            alpha: 1.0,
            opaque: true,
            dataspace: Dataspace::SRGB,
            hdr: HdrMetadata::default(),
        }
    }

    #[test]
    fn a_new_surface_control_sets_everything() {
        let mut ops = Vec::new();
        diff(None, &props(), &mut ops);
        let Placement::Visible(geometry) = props().placement else {
            unreachable!()
        };
        assert_eq!(
            ops,
            [
                Op::Z(1),
                Op::Visible(true),
                Op::Geometry(geometry),
                Op::Alpha(1.0),
                Op::Opaque(true),
                Op::Dataspace(Dataspace::SRGB),
                Op::Hdr(HdrMetadata::default()),
            ]
        );
    }

    #[test]
    fn a_transaction_sets_only_what_changed() {
        let prev = props();
        let mut ops = Vec::new();
        diff(Some(&prev), &prev, &mut ops);
        assert!(ops.is_empty(), "{ops:?}");

        let hdr = HdrMetadata {
            mastering: Some(MasteringDisplay {
                red: [0.708, 0.292],
                green: [0.170, 0.797],
                blue: [0.131, 0.046],
                white: [0.3127, 0.3290],
                max_luminance: 1000.0,
                min_luminance: 0.0001,
            }),
            content_light: Some(ContentLight {
                max_content: 1000.0,
                max_frame_average: 400.0,
            }),
        };
        let next = Properties {
            z: 3,
            alpha: 0.5,
            hdr,
            ..prev
        };
        diff(Some(&prev), &next, &mut ops);
        assert_eq!(ops, [Op::Z(3), Op::Alpha(0.5), Op::Hdr(hdr)]);

        ops.clear();
        let hidden = Properties {
            placement: Placement::Hidden,
            ..prev
        };
        diff(Some(&prev), &hidden, &mut ops);
        assert_eq!(ops, [Op::Visible(false)]);

        ops.clear();
        diff(Some(&hidden), &prev, &mut ops);
        let Placement::Visible(geometry) = prev.placement else {
            unreachable!()
        };
        assert_eq!(ops, [Op::Visible(true), Op::Geometry(geometry)]);
    }
}
