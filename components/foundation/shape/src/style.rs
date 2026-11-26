//! Visual shadow and transform metadata types (`Shadow`, `Scale`, `Rotation`, `Offset`).

use nami::{
    Computed, SignalExt,
    signal::{IntoComputed, IntoSignal},
};
use waterui_core::{Angle, IntoSignalF32, metadata::MetadataKey};
use waterui_graphics::color::Color;

use crate::{ClipShape, Shape};

/// Represents a shadow effect that can be applied to UI elements.
///
/// A shadow is defined by its color, offset from the original element, blur
/// radius, and an optional silhouette override. When no silhouette is given,
/// the shadow follows the caster's `.clip(...)` shape (falling back to a
/// rectangle when unclipped).
#[derive(Debug, Clone)]
pub struct Shadow {
    /// The color of the shadow, including alpha for opacity.
    color: Color,
    /// The horizontal offset of the shadow from the original element.
    offset_x: Computed<f32>,
    /// The vertical offset of the shadow from the original element.
    offset_y: Computed<f32>,
    /// The blur radius of the shadow in pixels.
    radius: Computed<f32>,
    /// Optional explicit caster silhouette override; `None` follows the caster's clip.
    silhouette: Option<ClipShape>,
}

impl Shadow {
    /// The color of the shadow, including alpha for opacity.
    #[must_use]
    pub const fn get_color(&self) -> &Color {
        &self.color
    }

    /// The horizontal offset of the shadow from the original element.
    #[must_use]
    pub const fn offset_x(&self) -> &Computed<f32> {
        &self.offset_x
    }

    /// The vertical offset of the shadow from the original element.
    #[must_use]
    pub const fn offset_y(&self) -> &Computed<f32> {
        &self.offset_y
    }

    /// The blur radius of the shadow in pixels.
    #[must_use]
    pub const fn radius(&self) -> &Computed<f32> {
        &self.radius
    }

    /// Optional explicit silhouette override; `None` follows the caster's clip.
    #[must_use]
    pub const fn get_silhouette(&self) -> Option<&ClipShape> {
        self.silhouette.as_ref()
    }
}

impl MetadataKey for Shadow {}

impl Shadow {
    /// Sets the shadow color.
    #[must_use]
    pub fn color(mut self, color: impl IntoSignal<Color> + 'static) -> Self {
        self.color = Color::from_signal(color);
        self
    }

    /// Sets the shadow offset along the x and y axes.
    #[must_use]
    pub fn offset(mut self, x: impl IntoSignalF32, y: impl IntoSignalF32) -> Self {
        self.offset_x = x.into_signal_f32().computed();
        self.offset_y = y.into_signal_f32().computed();
        self
    }

    /// Overrides the caster's silhouette with an explicit shape.
    #[must_use]
    pub fn silhouette(mut self, silhouette: impl Shape) -> Self {
        self.silhouette = Some(ClipShape::new(silhouette));
        self
    }

    /// Creates a black shadow with the given blur radius and zero offset,
    /// inheriting the caster's clip silhouette.
    #[must_use]
    pub fn new(radius: impl IntoSignalF32) -> Self {
        let zero = 0.0_f32.into_signal_f32().computed();
        Self {
            color: Color::srgb(0, 0, 0),
            offset_x: zero.clone(),
            offset_y: zero,
            radius: radius.into_signal_f32().computed(),
            silhouette: None,
        }
    }

    /// Creates a black shadow with the same offset and blur radius in both directions,
    /// inheriting the caster's clip silhouette.
    #[must_use]
    pub fn splat(value: impl IntoSignalF32) -> Self {
        let radius = value.into_signal_f32().computed();
        Self {
            color: Color::srgb(0, 0, 0),
            offset_x: radius.clone(),
            offset_y: radius.clone(),
            radius,
            silhouette: None,
        }
    }
}

impl Default for Shadow {
    /// A moderate black shadow slightly below the caster.
    fn default() -> Self {
        Self::new(4.0).offset(0.0, 2.0)
    }
}

/// A 2D vector with x and y components.
///
/// This type is used to represent positions, sizes, and offsets
/// in the UI coordinate system.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Vector<T> {
    /// The x component of the vector.
    pub x: T,
    /// The y component of the vector.
    pub y: T,
}

impl<T> Vector<T> {
    /// Creates a new vector with the given x and y components.
    ///
    /// # Arguments
    ///
    /// * `x` - The x component
    /// * `y` - The y component
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }

    /// Creates a vector with both components set to the same value.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to set for both x and y components
    pub const fn splat(value: T) -> Self
    where
        T: Copy,
    {
        Self { x: value, y: value }
    }

    /// Maps the components of the vector using the provided function.
    ///
    /// # Arguments
    /// * `f` - A function that takes a component and returns a new value
    ///
    /// # Returns
    /// A new vector with the mapped components
    pub fn map<U, F: FnMut(T) -> U>(self, mut f: F) -> Vector<U> {
        Vector {
            x: f(self.x),
            y: f(self.y),
        }
    }
}

// ============================================================================
// Transform Components (Scale, Rotation, Offset)
// ============================================================================

/// Anchor point for transforms, specified as normalized coordinates.
/// (0.0, 0.0) = top-left, (0.5, 0.5) = center, (1.0, 1.0) = bottom-right
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    /// X coordinate (0.0 = left, 0.5 = center, 1.0 = right)
    pub x: f32,
    /// Y coordinate (0.0 = top, 0.5 = center, 1.0 = bottom)
    pub y: f32,
}

impl Anchor {
    /// Center anchor point (0.5, 0.5)
    pub const CENTER: Self = Self { x: 0.5, y: 0.5 };
    /// Top-left anchor point (0.0, 0.0)
    pub const TOP_LEFT: Self = Self { x: 0.0, y: 0.0 };
    /// Top-right anchor point (1.0, 0.0)
    pub const TOP_RIGHT: Self = Self { x: 1.0, y: 0.0 };
    /// Bottom-left anchor point (0.0, 1.0)
    pub const BOTTOM_LEFT: Self = Self { x: 0.0, y: 1.0 };
    /// Bottom-right anchor point (1.0, 1.0)
    pub const BOTTOM_RIGHT: Self = Self { x: 1.0, y: 1.0 };

    /// Creates a custom anchor point.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl Default for Anchor {
    fn default() -> Self {
        Self::CENTER
    }
}

// ============================================================================
// Scale Metadata
// ============================================================================

/// Scale transform that can be applied to views.
///
/// Scales are purely visual and do not affect layout calculations.
/// The scale is applied around the specified anchor point.
#[derive(Debug, Clone)]
pub struct Scale {
    /// Scale factor along X axis (1.0 = no scale)
    x: Computed<f32>,
    /// Scale factor along Y axis (1.0 = no scale)
    y: Computed<f32>,
    /// Anchor point for the scale transform
    anchor: Anchor,
}

impl Scale {
    /// Scale factor along X axis (1.0 = no scale)
    #[must_use]
    pub const fn x(&self) -> &Computed<f32> {
        &self.x
    }

    /// Scale factor along Y axis (1.0 = no scale)
    #[must_use]
    pub const fn y(&self) -> &Computed<f32> {
        &self.y
    }

    /// Anchor point for the scale transform
    #[must_use]
    pub const fn anchor(&self) -> Anchor {
        self.anchor
    }
}

impl MetadataKey for Scale {}

impl Scale {
    /// Creates a uniform scale transform from center.
    #[must_use]
    pub fn uniform(factor: impl IntoSignalF32) -> Self {
        let factor = factor.into_signal_f32();
        Self {
            x: factor.clone().computed(),
            y: factor.computed(),
            anchor: Anchor::CENTER,
        }
    }

    /// Creates a uniform scale transform from a specific anchor point.
    #[must_use]
    pub fn uniform_from(factor: impl IntoSignalF32, anchor: Anchor) -> Self {
        let factor = factor.into_signal_f32();
        Self {
            x: factor.clone().computed(),
            y: factor.computed(),
            anchor,
        }
    }

    /// Creates a non-uniform scale transform from center.
    #[must_use]
    pub fn xy(x: impl IntoSignalF32, y: impl IntoSignalF32) -> Self {
        Self {
            x: x.into_signal_f32().computed(),
            y: y.into_signal_f32().computed(),
            anchor: Anchor::CENTER,
        }
    }

    /// Creates a non-uniform scale transform from a specific anchor point.
    #[must_use]
    pub fn xy_from(x: impl IntoSignalF32, y: impl IntoSignalF32, anchor: Anchor) -> Self {
        Self {
            x: x.into_signal_f32().computed(),
            y: y.into_signal_f32().computed(),
            anchor,
        }
    }
}

// ============================================================================
// Rotation Metadata
// ============================================================================

/// Rotation transform that can be applied to views.
///
/// Rotations are purely visual and do not affect layout calculations.
/// The rotation is applied around the specified anchor point.
#[derive(Debug, Clone)]
pub struct Rotation {
    /// Rotation angle (positive = clockwise)
    angle: Computed<Angle>,
    /// Anchor point for the rotation transform
    anchor: Anchor,
}

impl Rotation {
    /// Rotation angle (positive = clockwise)
    #[must_use]
    pub const fn angle(&self) -> &Computed<Angle> {
        &self.angle
    }

    /// Anchor point for the rotation transform
    #[must_use]
    pub const fn anchor(&self) -> Anchor {
        self.anchor
    }
}

impl MetadataKey for Rotation {}

impl Rotation {
    /// Creates a rotation transform around center.
    #[must_use]
    pub fn new(angle: impl IntoComputed<Angle>) -> Self {
        Self {
            angle: angle.into_computed(),
            anchor: Anchor::CENTER,
        }
    }

    /// Creates a rotation transform around a specific anchor point.
    #[must_use]
    pub fn from_anchor(angle: impl IntoComputed<Angle>, anchor: Anchor) -> Self {
        Self {
            angle: angle.into_computed(),
            anchor,
        }
    }
}

// ============================================================================
// Offset Metadata
// ============================================================================

/// Offset (translation) transform that can be applied to views.
///
/// Offsets are purely visual and do not affect layout calculations.
/// This is a simple translation with no anchor point needed.
#[derive(Debug, Clone)]
pub struct Offset {
    /// Offset along X axis in points
    x: Computed<f32>,
    /// Offset along Y axis in points
    y: Computed<f32>,
}

impl MetadataKey for Offset {}

impl Offset {
    /// Creates an offset transform.
    #[must_use]
    pub fn new(x: impl IntoSignalF32, y: impl IntoSignalF32) -> Self {
        Self {
            x: x.into_signal_f32().computed(),
            y: y.into_signal_f32().computed(),
        }
    }

    /// Offset along X axis in points.
    #[must_use]
    pub const fn x(&self) -> &Computed<f32> {
        &self.x
    }

    /// Offset along Y axis in points.
    #[must_use]
    pub const fn y(&self) -> &Computed<f32> {
        &self.y
    }
}

#[cfg(test)]
mod tests {
    use nami::Signal;
    use waterui_core::Angle;

    use super::{Anchor, Offset, Rotation, Scale, Shadow};
    use crate::RoundedRectangle;

    #[test]
    fn style_transforms_and_shadow_clone_and_expose_accessors() {
        let shadow = Shadow::new(6.0)
            .offset(1.0, 2.0)
            .silhouette(RoundedRectangle::new(8.0));
        let shadow_clone = shadow.clone();
        assert!((shadow.radius().snapshot() - 6.0).abs() < f32::EPSILON);
        assert!((shadow_clone.radius().snapshot() - 6.0).abs() < f32::EPSILON);
        assert!((shadow_clone.offset_x().snapshot() - 1.0).abs() < f32::EPSILON);
        assert!((shadow_clone.offset_y().snapshot() - 2.0).abs() < f32::EPSILON);
        assert!(shadow_clone.get_silhouette().is_some());

        let scale = Scale::xy_from(1.25, 0.75, Anchor::TOP_LEFT);
        let scale_clone = scale.clone();
        assert_eq!(scale.anchor(), Anchor::TOP_LEFT);
        assert!((scale_clone.x().snapshot() - 1.25).abs() < f32::EPSILON);
        assert!((scale_clone.y().snapshot() - 0.75).abs() < f32::EPSILON);
        assert_eq!(scale_clone.anchor(), Anchor::TOP_LEFT);

        let rotation = Rotation::from_anchor(Angle::degrees(30.0), Anchor::BOTTOM_RIGHT);
        let rotation_clone = rotation.clone();
        assert_eq!(rotation.anchor(), Anchor::BOTTOM_RIGHT);
        assert_eq!(rotation_clone.angle().snapshot(), Angle::degrees(30.0));
        assert_eq!(rotation_clone.anchor(), Anchor::BOTTOM_RIGHT);

        let offset = Offset::new(12.0, -4.0);
        let offset_clone = offset.clone();
        assert!((offset.x().snapshot() - 12.0).abs() < f32::EPSILON);
        assert!((offset_clone.x().snapshot() - 12.0).abs() < f32::EPSILON);
        assert!((offset_clone.y().snapshot() - (-4.0)).abs() < f32::EPSILON);
    }
}
