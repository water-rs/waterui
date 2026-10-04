//! CPU-side instance data matching `shader.wgsl` byte for byte.

use bytemuck::{Pod, Zeroable};

/// Fill a shape.
pub const KIND_FILL: u32 = 0;
/// Stroke via `coverage(outer) - coverage(inner)`.
pub const KIND_STROKE_OFFSET: u32 = 1;
/// Stroke via `coverage(d - hw) - coverage(d + hw)`.
pub const KIND_STROKE_DIST: u32 = 2;
/// A Gaussian-blurred rounded box.
pub const KIND_SHADOW: u32 = 3;
/// Coverage sampled from the glyph atlas.
pub const KIND_GLYPH: u32 = 4;
/// A device-space run of fully covered columns from a path strip.
pub const KIND_SPAN: u32 = 5;
/// An atlas cell with an explicit full-coverage horizontal interval.
/// Solid paint leaves `meta[2]` available for its two u16 column offsets.
pub const KIND_REGION: u32 = 6;

/// A single colour.
pub const PAINT_SOLID: u32 = 0;
/// A linear gradient.
pub const PAINT_LINEAR: u32 = 1;
/// A two-point radial gradient.
pub const PAINT_RADIAL: u32 = 2;
/// A composite: sample the bound scratch texture at the device pixel.
pub const PAINT_TEXTURE: u32 = 3;
/// A sweep (conic) gradient.
pub const PAINT_SWEEP: u32 = 4;
/// An image sampled manually from the bound image texture.
pub const PAINT_IMAGE: u32 = 5;
/// Paint coordinates are mapped by the two stop-buffer entries immediately
/// preceding `first_stop`; lower bits keep the underlying paint kind.
pub const PAINT_TRANSFORMED: u32 = 1 << 16;
/// Bilinear mesh patches in the shared paint buffer.
pub const PAINT_MESH: u32 = 6;
/// A backdrop member composite with a per-member effect: bilinear sample
/// of the bound capture, then the effect from `meta[3]`'s low bits.
pub const PAINT_BACKDROP: u32 = 7;

/// A projective composite (#84): drawn only by the projective pipeline,
/// which samples the layer's local image through the inverse homography
/// in the three stop rows at `first_stop`.
pub const PAINT_PROJECTIVE: u32 = 8;

/// Effect kinds packed into `meta[3]`'s low byte of a `PAINT_BACKDROP`
/// instance (`param stop count << 8`, flags still `<< 24`).
pub const EFFECT_COLOR: u32 = 1;
/// [`EFFECT_COLOR`]'s sibling: displace the sample along the edge normal.
pub const EFFECT_REFRACTION: u32 = 2;
/// A registered backdrop effect shader.
pub const EFFECT_SHADER: u32 = 3;
/// A rim highlight additive on the sample inside the clip edge.
pub const EFFECT_RIM: u32 = 4;
/// Smooth colour weights; low 16 bits still identify the mesh paint kind.
pub const PAINT_MESH_SMOOTH: u32 = 1 << 17;

/// Clamp the edge colours.
pub const EXTEND_PAD: u32 = 0;
/// Repeat the range.
pub const EXTEND_REPEAT: u32 = 1;
/// Repeat the range mirrored.
pub const EXTEND_REFLECT: u32 = 2;
/// Transparent outside the range.
pub const EXTEND_NONE: u32 = 3;

/// Stops stored in the working space.
pub const INTERP_WORKING: u32 = 0;
/// Stops stored sRGB-encoded.
pub const INTERP_SRGB: u32 = 1;

/// The instance's clip fields are live.
pub const FLAG_HAS_CLIP: u32 = 1;
/// The stroke has an inner edge.
pub const FLAG_HAS_INNER: u32 = 2;
/// The clip carries a coverage mask sampled from the atlas.
pub const FLAG_HAS_MASK: u32 = 4;
/// The clip mask is sampled from the bound mask texture instead of the
/// atlas.
pub const FLAG_MASK_TEXTURE: u32 = 8;
/// A `PAINT_TEXTURE` source stores sRGB-encoded premultiplied pixels
/// (the pass's declared [`cherenkov::BlendSpace::SrgbEncoded`] space)
/// rather than linear ones; without it the texture is linear.
pub const FLAG_TEX_SRGB: u32 = 0x10;
/// A `PAINT_TEXTURE` composite blends in the source texture's space:
/// the destination converts into it, the result converts back. Set when
/// the isolated plane's storage space differs from the pass's.
pub const FLAG_BLEND_SRC: u32 = 0x20;

/// The shader's blend-mode code for a [`cherenkov::BlendMode`]; `0` keeps the
/// fixed-function source-over composite. Matches `blend_mode` in the WGSL.
pub const fn blend_code(mode: cherenkov::BlendMode) -> u32 {
    match mode {
        cherenkov::BlendMode::Normal => 0,
        cherenkov::BlendMode::Multiply => 1,
        cherenkov::BlendMode::Screen => 2,
        cherenkov::BlendMode::Overlay => 3,
        cherenkov::BlendMode::Darken => 4,
        cherenkov::BlendMode::Lighten => 5,
        cherenkov::BlendMode::ColorDodge => 6,
        cherenkov::BlendMode::ColorBurn => 7,
        cherenkov::BlendMode::HardLight => 8,
        cherenkov::BlendMode::SoftLight => 9,
        cherenkov::BlendMode::Difference => 10,
        cherenkov::BlendMode::Exclusion => 11,
        cherenkov::BlendMode::Hue => 12,
        cherenkov::BlendMode::Saturation => 13,
        cherenkov::BlendMode::Color => 14,
        cherenkov::BlendMode::Luminosity => 15,
        cherenkov::BlendMode::Clear => 16,
        cherenkov::BlendMode::Src => 17,
        cherenkov::BlendMode::Dst => 18,
        cherenkov::BlendMode::DestOver => 19,
        cherenkov::BlendMode::SrcIn => 20,
        cherenkov::BlendMode::DestIn => 21,
        cherenkov::BlendMode::SrcOut => 22,
        cherenkov::BlendMode::DestOut => 23,
        cherenkov::BlendMode::SrcAtop => 24,
        cherenkov::BlendMode::DestAtop => 25,
        cherenkov::BlendMode::Xor => 26,
        cherenkov::BlendMode::PlusLighter => 27,
    }
}

/// A rounded box centred at the origin, mirroring the WGSL `Shape`.
///
/// `radii` are the corner radii along x in the order top-left, top-right,
/// bottom-right, bottom-left; the radius along y is `radius * aspect`.
/// `exponent` is the Lamé exponent of the corner curve: 2.0 for a circular
/// or elliptical corner, larger for a continuous corner.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Shape {
    /// Half extents of the box.
    pub half: [f32; 2],
    /// Corner radius aspect (y radius / x radius).
    pub aspect: f32,
    /// Lamé exponent of the corner curve; 0.0 means "none".
    pub exponent: f32,
    /// Corner radii along x: top-left, top-right, bottom-right, bottom-left.
    pub radii: [f32; 4],
}

impl Shape {
    /// A sharp box of half extents `half`.
    pub const fn rect(half: [f32; 2]) -> Self {
        Self {
            half,
            aspect: 1.0,
            exponent: 2.0,
            radii: [0.0; 4],
        }
    }
}

/// One instanced quad, mirroring the WGSL `Instance`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Instance {
    /// Local-to-device affine: `[a, b, c, d, e, f, 0, 0]`.
    pub affine: [f32; 8],
    /// Quad rectangle `(x0, y0, x1, y1)`, local space except `KIND_GLYPH`
    /// `KIND_SPAN` and `KIND_REGION`, where it is a device-space rectangle.
    pub bounds: [f32; 4],
    /// The shape being drawn.
    pub shape: Shape,
    /// The inner shape of an offset stroke.
    pub inner: Shape,
    /// Device-to-clip-local affine.
    pub clip_inv: [f32; 8],
    /// The clip shape. A masked clip is a sharp rect, so `aspect` and
    /// `exponent` (unused by its SDF) carry the mask cell size.
    pub clip: Shape,
    /// Straight-alpha working-space colour.
    pub color: [f32; 4],
    /// Linear: start.xy, end.xy. Radial: start centre.xy, end centre.xy.
    /// Sweep: centre.xy. Image: local→image affine `[a, b, c, d]`.
    /// `PAINT_TEXTURE`: source region origin.xy.
    pub grad: [f32; 4],
    /// Radial: start radius, end radius. Sweep: start angle, end angle.
    /// Image: local→image affine `[e, f]` and image `[w, h]`.
    pub grad2: [f32; 4],
    /// Glyph/cell: atlas cell origin in texels. zw: mask atlas cell origin.
    pub uv: [f32; 4],
    /// x: stroke half width or shadow sigma. y: opacity. zw: mask device
    /// origin.
    pub params: [f32; 4],
    /// `[kind, paint, first_stop, count | interp<<16 | extend<<20 | flags<<24]`.
    /// For `PAINT_IMAGE`: `extend_x | extend_y<<4 | sampling<<8 | flags<<24`
    /// (`sampling`: 0 nearest, 1 bilinear). For a blended `PAINT_TEXTURE`
    /// composite: `blend_code<<16 | flags<<24` (the low 16 bits are free
    /// there). For `PAINT_BACKDROP`: `effect_kind | stop_count<<8 |
    /// flags<<24`.
    /// For `KIND_REGION`, `first_stop` holds the full-column interval instead.
    pub meta: [u32; 4],
}

impl Instance {
    /// An instance of `kind` with no paint resources.
    pub const fn new(kind: u32) -> Self {
        Self {
            affine: [0.0; 8],
            bounds: [0.0; 4],
            shape: Shape::rect([0.0; 2]),
            inner: Shape::rect([0.0; 2]),
            clip_inv: [0.0; 8],
            clip: Shape::rect([0.0; 2]),
            color: [0.0; 4],
            grad: [0.0; 4],
            grad2: [0.0; 4],
            uv: [0.0; 4],
            params: [0.0, 1.0, 0.0, 0.0],
            meta: [kind, PAINT_SOLID, 0, 0],
        }
    }
}

/// One gradient stop, mirroring the WGSL `Stop`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Stop {
    /// Straight-alpha colour in the interpolation space.
    pub color: [f32; 4],
    /// Position along the gradient, 0 to 1.
    pub offset: f32,
    /// Padding.
    pub pad: [f32; 3],
}

/// Per-pass constants, mirroring the WGSL `Globals`; one entry per pass
/// at a 256-byte stride behind a dynamic uniform offset.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Globals {
    /// Target size in pixels (the pass's region).
    pub size: [f32; 2],
    /// Device-space origin of the target region.
    pub origin: [f32; 2],
    /// The space the pass's target stores: `0` linear, `1` sRGB-encoded.
    pub space: u32,
    /// Padding to the uniform alignment.
    pub pad: u32,
    /// Device-space origin of the physical attachment grid.
    pub attachment_origin: [f32; 2],
}

/// Converts a kurbo affine into the shader's `[a, b, c, d, e, f, 0, 0]`.
#[expect(clippy::cast_possible_truncation, reason = "instance data is f32")]
pub const fn affine(transform: kurbo::Affine) -> [f32; 8] {
    let [c0, c1, c2, c3, c4, c5] = transform.as_coeffs();
    [
        c0 as f32, c1 as f32, c2 as f32, c3 as f32, c4 as f32, c5 as f32, 0.0, 0.0,
    ]
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use super::*;

    #[test]
    fn layouts_match_the_shader() {
        assert_eq!(size_of::<Instance>(), 272);
        assert_eq!(size_of::<Shape>(), 32);
        assert_eq!(size_of::<Stop>(), 32);
        assert_eq!(size_of::<Globals>(), 32);
        assert_eq!(
            super::super::bindings::ENGINE_GROUP0[0].min_size,
            size_of::<Globals>() as u64
        );
    }
}
