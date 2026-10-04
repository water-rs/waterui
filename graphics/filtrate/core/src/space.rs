//! Colour spaces: the working space's constants and a stage's operating
//! space.

/// The colour space a stage operates in.
///
/// Executors hand every stage premultiplied colours in the space it declares
/// and convert around it. Extended values (negative components, values above
/// one) stay extended through every conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OperatingSpace {
    /// The linear working space, linear Display P3.
    #[default]
    Working,
    /// sRGB: sRGB primaries with the sRGB transfer function applied, the
    /// space CSS filter functions are defined in.
    Srgb,
}

/// The working-space constants a stage receives through its `space`
/// argument, so filters never hard-code another space's coefficients.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkingSpace {
    /// The luma coefficients: the Y row of the working space's RGB to XYZ
    /// matrix.
    pub luma: [f32; 3],
}

impl WorkingSpace {
    /// Linear Display P3 (D65), the working space.
    pub const LINEAR_DISPLAY_P3: Self = Self {
        luma: [0.228_974_6, 0.691_738_5, 0.079_286_9],
    };
}
