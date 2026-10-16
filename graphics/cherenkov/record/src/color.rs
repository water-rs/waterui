//! Colour: typed colour spaces, and the linear Display P3 working space.

use color::{AlphaColor, ColorSpaceTag, DynamicColor};
use filtrate_core::space::{P3_TO_SRGB, SRGB_TO_P3, transform};
use nami_core::Signal;
use nami_core::watcher::Context;

pub use color::{ColorSpace, DisplayP3, LinearSrgb, Rec2020, Srgb};

/// Linear-light Display P3, the engine's working space.
///
/// Every colour the engine composites is converted into this space. Values
/// above 1.0 are HDR, relative to SDR white.
#[derive(Clone, Copy, Debug)]
pub struct LinearDisplayP3;

// The matrices between linear Display P3 and linear sRGB are the engine's
// one definition, filtrate-core's `space`; they round the same derivation
// the `color` crate's `DisplayP3` uses, so both crates agree on the gamut.
impl ColorSpace for LinearDisplayP3 {
    const IS_LINEAR: bool = true;

    const WHITE_COMPONENTS: [f32; 3] = [1., 1., 1.];

    fn to_linear_srgb(src: [f32; 3]) -> [f32; 3] {
        transform(&P3_TO_SRGB, src)
    }

    fn from_linear_srgb(src: [f32; 3]) -> [f32; 3] {
        transform(&SRGB_TO_P3, src)
    }

    fn clip([r, g, b]: [f32; 3]) -> [f32; 3] {
        [r.clamp(0., 1.), g.clamp(0., 1.), b.clamp(0., 1.)]
    }
}

/// A colour in the working space: linear Display P3, straight alpha, extended
/// range.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WorkingColor {
    /// Red, green, blue in linear Display P3, then alpha.
    pub components: [f32; 4],
}

impl WorkingColor {
    /// Opaque black.
    pub const BLACK: Self = Self::new([0., 0., 0., 1.]);
    /// Opaque SDR white.
    pub const WHITE: Self = Self::new([1., 1., 1., 1.]);
    /// Fully transparent.
    pub const TRANSPARENT: Self = Self::new([0., 0., 0., 0.]);

    /// Creates a working-space colour from linear Display P3 components and alpha.
    #[must_use]
    pub const fn new(components: [f32; 4]) -> Self {
        Self { components }
    }

    /// Returns this colour with its alpha replaced.
    #[must_use]
    pub const fn with_alpha(self, alpha: f32) -> Self {
        let [r, g, b, _] = self.components;
        Self::new([r, g, b, alpha])
    }
}

/// A colour in the colour space `CS`, known at compile time.
///
/// The conversion into the working space is monomorphised per colour space,
/// so it has no run-time dispatch. Components may exceed 1.0; in a linear
/// space that is HDR relative to SDR white.
#[derive(Clone, Copy, Debug)]
pub struct Color<CS: ColorSpace>(AlphaColor<CS>);

impl<CS: ColorSpace> Color<CS> {
    /// Creates a colour from its three colour components and alpha.
    #[must_use]
    pub const fn new(components: [f32; 4]) -> Self {
        Self(AlphaColor::new(components))
    }

    /// The colour components and alpha in `CS`.
    #[must_use]
    pub const fn components(self) -> [f32; 4] {
        self.0.components
    }

    /// Converts into the working space.
    #[must_use]
    pub fn to_working(self) -> WorkingColor {
        WorkingColor::new(self.0.convert::<LinearDisplayP3>().components)
    }
}

impl<CS: ColorSpace> From<AlphaColor<CS>> for Color<CS> {
    fn from(color: AlphaColor<CS>) -> Self {
        Self(color)
    }
}

impl<CS: ColorSpace> From<Color<CS>> for WorkingColor {
    fn from(color: Color<CS>) -> Self {
        color.to_working()
    }
}

/// A colour whose colour space is known only at run time, such as a parsed CSS
/// colour.
#[derive(Clone, Copy, Debug)]
pub struct DynColor(DynamicColor);

impl DynColor {
    /// Wraps a dynamic colour.
    #[must_use]
    pub const fn new(color: DynamicColor) -> Self {
        Self(color)
    }

    /// Converts into the working space.
    #[must_use]
    pub fn to_working(self) -> WorkingColor {
        let linear = self
            .0
            .convert(ColorSpaceTag::LinearSrgb)
            .to_alpha_color::<LinearSrgb>();
        WorkingColor::new(linear.convert::<LinearDisplayP3>().components)
    }
}

impl From<DynamicColor> for DynColor {
    fn from(color: DynamicColor) -> Self {
        Self(color)
    }
}

impl From<DynColor> for WorkingColor {
    fn from(color: DynColor) -> Self {
        color.to_working()
    }
}

impl<CS: ColorSpace> Signal for Color<CS> {
    type Output = Self;
    type Guard = ();

    fn snapshot(&self) -> Self::Output {
        *self
    }

    fn watch(&self, _watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {}
}

nami_core::impl_constant!(DynColor, WorkingColor);

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: [f32; 4], expected: [f32; 4]) {
        for (a, e) in actual.iter().zip(expected) {
            assert!((a - e).abs() < 1e-5, "{actual:?} != {expected:?}");
        }
    }

    #[test]
    fn srgb_red_lands_inside_p3() {
        let red = Color::<Srgb>::new([1., 0., 0., 1.]).to_working();
        assert_close(red.components, [0.822_462, 0.033_194, 0.017_083, 1.]);
    }

    #[test]
    fn display_p3_is_only_linearized() {
        let red = Color::<DisplayP3>::new([1., 0., 0., 1.]).to_working();
        assert_close(red.components, [1., 0., 0., 1.]);
        let mid = Color::<DisplayP3>::new([0.5, 0.5, 0.5, 1.]).to_working();
        assert_close(mid.components, [0.214_041, 0.214_041, 0.214_041, 1.]);
    }

    #[test]
    fn hdr_values_survive_conversion() {
        let bright = Color::<LinearSrgb>::new([4., 4., 4., 1.]).to_working();
        assert_close(bright.components, [4., 4., 4., 1.]);
    }

    #[test]
    fn dynamic_colours_match_typed_colours() {
        let parsed = color::parse_color("color(display-p3 1 0 0)").expect("valid CSS colour");
        let dynamic = DynColor::new(parsed).to_working();
        let typed = Color::<DisplayP3>::new([1., 0., 0., 1.]).to_working();
        assert_close(dynamic.components, typed.components);
    }
}
