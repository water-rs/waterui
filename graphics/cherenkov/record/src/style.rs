//! Shadows and group styles.

use kurbo::Vec2;

use crate::color::WorkingColor;

/// A shadow cast by a shape.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Shadow {
    /// Standard deviation of the Gaussian blur, in the shape's units.
    pub sigma: f64,
    /// Offset of the shadow from the shape.
    pub offset: Vec2,
    /// How far the shape grows (positive) or shrinks (negative) before blurring.
    pub spread: f64,
    /// Colour of the shadow.
    pub color: WorkingColor,
}

impl Shadow {
    /// Creates an unoffset shadow with no spread.
    #[must_use]
    pub fn new(sigma: f64, color: impl Into<WorkingColor>) -> Self {
        Self {
            sigma,
            offset: Vec2::ZERO,
            spread: 0.,
            color: color.into(),
        }
    }

    /// Sets the offset.
    #[must_use]
    pub fn offset(self, offset: impl Into<Vec2>) -> Self {
        Self {
            offset: offset.into(),
            ..self
        }
    }

    /// Sets the spread.
    #[must_use]
    pub const fn spread(self, spread: f64) -> Self {
        Self { spread, ..self }
    }
}

/// How a group's content blends with what lies beneath it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BlendMode {
    /// Source over.
    #[default]
    Normal,
    /// Multiply.
    Multiply,
    /// Screen.
    Screen,
    /// Overlay.
    Overlay,
    /// Darken.
    Darken,
    /// Lighten.
    Lighten,
    /// Colour dodge.
    ColorDodge,
    /// Colour burn.
    ColorBurn,
    /// Hard light.
    HardLight,
    /// Soft light.
    SoftLight,
    /// Difference.
    Difference,
    /// Exclusion.
    Exclusion,
    /// Hue.
    Hue,
    /// Saturation.
    Saturation,
    /// Colour.
    Color,
    /// Luminosity.
    Luminosity,
    /// Both source and destination are cleared.
    Clear,
    /// The source replaces the destination.
    Src,
    /// The destination replaces the source (source discarded).
    Dst,
    /// The destination is placed over the source.
    DestOver,
    /// The parts of the source that overlap the destination.
    SrcIn,
    /// The parts of the destination that overlap the source.
    DestIn,
    /// The parts of the source outside the destination.
    SrcOut,
    /// The parts of the destination outside the source.
    DestOut,
    /// The parts of the source overlapping the destination replace it.
    SrcAtop,
    /// The parts of the destination overlapping the source replace it.
    DestAtop,
    /// The non-overlapping regions of source and destination.
    Xor,
    /// Source and destination are summed without clamping.
    PlusLighter,
}

impl BlendMode {
    /// Whether a transparent source changes the destination under this
    /// operator — `Clear`, `Src`, `SrcIn`, `SrcOut`, `DestIn` and
    /// `DestAtop` — instead of leaving it as it is. Such a composite covers
    /// the whole clip it applies under, not only the source's pixels.
    #[must_use]
    pub const fn is_destructive(self) -> bool {
        matches!(
            self,
            Self::Clear | Self::Src | Self::SrcIn | Self::SrcOut | Self::DestIn | Self::DestAtop
        )
    }
}

/// The space in which a group blends, or in which a surface stores its
/// pixels ([`Shared::set_storage_space`](crate::surface::Shared::set_storage_space)).
///
/// A layer or group that declares no space composites in the space of
/// the level it composites into, so a surface storing
/// [`BlendSpace::SrgbEncoded`] blends its plain translucent paints as
/// sRGB-space alpha blends without isolating them. One that declares a
/// space unlike that level's isolates and composites across the spaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BlendSpace {
    /// The linear working space.
    #[default]
    Linear,
    /// sRGB-encoded values, for web compatibility.
    SrgbEncoded,
}

/// A filter chain registered with the render target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FilterId {
    raw: u64,
    coordinate_remap: bool,
}

impl FilterId {
    /// Creates an identifier from a backend-assigned raw value, for a
    /// filter that is not a coordinate remap.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self {
            raw,
            coordinate_remap: false,
        }
    }

    /// Declares whether the filter is a pure coordinate remap
    /// (`filtrate_core::Filter::COORDINATE_REMAP`), set by whoever
    /// registers it from the filter's type.
    #[must_use]
    pub const fn with_coordinate_remap(self, coordinate_remap: bool) -> Self {
        Self {
            coordinate_remap,
            ..self
        }
    }

    /// The raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.raw
    }

    /// Whether the filter is a pure coordinate remap: it commutes with
    /// source-over compositing up to edge filtering, so a group it filters
    /// may be filtered part by part in paint order.
    #[must_use]
    pub const fn is_coordinate_remap(self) -> bool {
        self.coordinate_remap
    }
}

/// The isolation of a group: its opacity, how it blends and its filter.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Group {
    /// Opacity applied to the composited group.
    pub opacity: f32,
    /// Blend mode of the group onto its parent.
    pub blend: BlendMode,
    /// The space in which the group blends; `None` blends in the space of
    /// the level the group composites into.
    pub blend_space: Option<BlendSpace>,
    /// Filter applied to the group.
    pub filter: Option<FilterId>,
}

impl Group {
    /// An opaque, normally blended group with no filter, blending in the
    /// space it composites into.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            opacity: 1.,
            blend: BlendMode::Normal,
            blend_space: None,
            filter: None,
        }
    }

    /// Sets the opacity.
    #[must_use]
    pub const fn opacity(self, opacity: f32) -> Self {
        Self { opacity, ..self }
    }

    /// Sets the blend mode.
    #[must_use]
    pub const fn blend(self, blend: BlendMode) -> Self {
        Self { blend, ..self }
    }

    /// Sets the blend space: members composite with each other in this
    /// space, and the group composites onto its backdrop in it. A space
    /// unlike the enclosing level's isolates the group.
    #[must_use]
    pub const fn blend_space(self, blend_space: BlendSpace) -> Self {
        Self {
            blend_space: Some(blend_space),
            ..self
        }
    }

    /// Sets the filter.
    #[must_use]
    pub const fn filter(self, filter: FilterId) -> Self {
        Self {
            filter: Some(filter),
            ..self
        }
    }
}

impl Default for Group {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::animation::AnimLanes for Shadow {
    fn anim_lanes(&self, _target: &Self) -> Option<Box<[f64]>> {
        let mut lanes = Vec::with_capacity(8);
        lanes.extend([self.sigma, self.offset.x, self.offset.y, self.spread]);
        lanes.extend(self.color.components.iter().map(|&c| f64::from(c)));
        Some(lanes.into_boxed_slice())
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "f64 lanes quantize to f32 shadow colour"
    )]
    fn with_lanes(&self, lanes: &[f64]) -> Self {
        Self {
            // A spring that overshoots its target passes below a zero blur,
            // which the engine rejects: a blur has no negative extent.
            sigma: lanes[0].max(0.0),
            offset: Vec2::new(lanes[1], lanes[2]),
            spread: lanes[3],
            color: WorkingColor::new([
                lanes[4] as f32,
                lanes[5] as f32,
                lanes[6] as f32,
                lanes[7] as f32,
            ]),
        }
    }
}

impl crate::animation::AnimLanes for Group {
    fn anim_lanes(&self, target: &Self) -> Option<Box<[f64]>> {
        (self.blend == target.blend
            && self.blend_space == target.blend_space
            && self.filter == target.filter)
            .then(|| Box::new([f64::from(self.opacity)]) as Box<[f64]>)
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "f64 lanes quantize to f32 opacity"
    )]
    fn with_lanes(&self, lanes: &[f64]) -> Self {
        Self {
            opacity: lanes[0] as f32,
            ..*self
        }
    }
}

nami_core::impl_constant!(Shadow, Group);

#[cfg(test)]
mod tests {
    use super::Shadow;
    use crate::animation::AnimLanes as _;
    use crate::color::WorkingColor;

    /// A spring overshooting a shadow's blur past zero leaves it at zero.
    #[test]
    fn an_overshooting_blur_stops_at_zero() {
        let shadow = Shadow::new(2.0, WorkingColor::new([0.0, 0.0, 0.0, 0.3]));
        let mut lanes = shadow
            .anim_lanes(&shadow)
            .expect("a shadow animates by lanes");
        lanes[0] = -0.4;
        assert!(shadow.with_lanes(&lanes).sigma.abs() < f64::EPSILON);
    }
}
