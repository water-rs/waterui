//! Absolute positioning layout for WaterUI.
//!
//! This module provides an `Absolute` container that fills available space
//! and allows children to be positioned at specific coordinates within.
//!
//! # Example
//!
//! ```rust,ignore
//! use waterui_layout::{absolute, UnitPoint, PositionExt};
//!
//! absolute((
//!     Color::gray(),  // Fills container
//!     text("Center").position_in(UnitPoint::CENTER),
//!     badge.position_in_offset(
//!         UnitPoint::TOP_TRAILING,
//!         UnitPoint::TOP_TRAILING,
//!         -8.0, 8.0
//!     ),
//! ))
//! ```

use alloc::vec::Vec;
use core::fmt;

use nami::{Computed, Signal, signal::IntoComputed};
use waterui_core::{View, view::TupleViews};

use crate::{
    Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView,
    container::FixedContainer, stack::Alignment,
};

// ============================================================================
// UnitPoint - Normalized coordinates for positioning
// ============================================================================

/// Normalized coordinates (0.0-1.0) for positioning.
///
/// Used to specify both anchor points on views and target positions in parent.
/// Values outside 0.0-1.0 are valid and will position outside bounds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UnitPoint {
    /// X coordinate (0.0 = left edge, 1.0 = right edge)
    pub x: f32,
    /// Y coordinate (0.0 = top edge, 1.0 = bottom edge)
    pub y: f32,
}

impl UnitPoint {
    /// Top-left corner (0.0, 0.0)
    pub const TOP_LEADING: Self = Self { x: 0.0, y: 0.0 };
    /// Top center (0.5, 0.0)
    pub const TOP: Self = Self { x: 0.5, y: 0.0 };
    /// Top-right corner (1.0, 0.0)
    pub const TOP_TRAILING: Self = Self { x: 1.0, y: 0.0 };
    /// Left center (0.0, 0.5)
    pub const LEADING: Self = Self { x: 0.0, y: 0.5 };
    /// Center (0.5, 0.5)
    pub const CENTER: Self = Self { x: 0.5, y: 0.5 };
    /// Right center (1.0, 0.5)
    pub const TRAILING: Self = Self { x: 1.0, y: 0.5 };
    /// Bottom-left corner (0.0, 1.0)
    pub const BOTTOM_LEADING: Self = Self { x: 0.0, y: 1.0 };
    /// Bottom center (0.5, 1.0)
    pub const BOTTOM: Self = Self { x: 0.5, y: 1.0 };
    /// Bottom-right corner (1.0, 1.0)
    pub const BOTTOM_TRAILING: Self = Self { x: 1.0, y: 1.0 };

    /// Creates a custom unit point.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl From<Alignment> for UnitPoint {
    fn from(alignment: Alignment) -> Self {
        match alignment {
            Alignment::TopLeading => Self::TOP_LEADING,
            Alignment::Top => Self::TOP,
            Alignment::TopTrailing => Self::TOP_TRAILING,
            Alignment::Leading => Self::LEADING,
            Alignment::Center => Self::CENTER,
            Alignment::Trailing => Self::TRAILING,
            Alignment::BottomLeading => Self::BOTTOM_LEADING,
            Alignment::Bottom => Self::BOTTOM,
            Alignment::BottomTrailing => Self::BOTTOM_TRAILING,
        }
    }
}

// ============================================================================
// AbsoluteLayout - Fills parent, gives all children full bounds
// ============================================================================

/// Layout that fills parent and gives each child full bounds.
///
/// Children are responsible for their own positioning (via `PositionedLayout`).
#[derive(Debug, Clone, Default)]
pub struct AbsoluteLayout;

impl Layout for AbsoluteLayout {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn size_that_fits(&self, proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
        // Take whatever space is offered, or infinity if unspecified
        Size::new(
            proposal.width.unwrap_or(f32::INFINITY),
            proposal.height.unwrap_or(f32::INFINITY),
        )
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        // Give every child the full bounds - they position themselves
        children.iter().map(|_| bounds).collect()
    }
}

// ============================================================================
// PositionTarget - Where to position the child
// ============================================================================

/// Target position for absolute positioning.
#[derive(Debug, Clone)]
pub enum PositionTarget {
    /// Absolute position in points (relative to parent origin)
    Absolute {
        /// X coordinate in points
        x: Computed<f32>,
        /// Y coordinate in points
        y: Computed<f32>,
    },
    /// Fractional position (0.0-1.0 of parent) plus offset
    Fractional {
        /// Fractional position in parent
        unit: UnitPoint,
        /// Additional X offset in points
        offset_x: Computed<f32>,
        /// Additional Y offset in points
        offset_y: Computed<f32>,
    },
}

// ============================================================================
// PositionedLayout - Positions a single child within received bounds
// ============================================================================

/// Layout that positions a single child within received bounds.
pub struct PositionedLayout {
    /// Anchor point on the child (0.0-1.0)
    pub anchor: UnitPoint,
    /// Target position in parent
    pub target: PositionTarget,
}

impl fmt::Debug for PositionedLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PositionedLayout")
            .field("anchor", &self.anchor)
            .finish_non_exhaustive()
    }
}

impl Layout for PositionedLayout {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn size_that_fits(&self, proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
        Size::new(
            proposal.width.unwrap_or(f32::INFINITY),
            proposal.height.unwrap_or(f32::INFINITY),
        )
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        children
            .iter()
            .map(|child| {
                let child_size = child.size_that_fits(ProposalSize::UNSPECIFIED);

                let (target_x, target_y) = match &self.target {
                    PositionTarget::Absolute { x, y } => {
                        (bounds.x() + x.get(), bounds.y() + y.get())
                    }
                    PositionTarget::Fractional {
                        unit,
                        offset_x,
                        offset_y,
                    } => (
                        bounds.x() + bounds.width() * unit.x + offset_x.get(),
                        bounds.y() + bounds.height() * unit.y + offset_y.get(),
                    ),
                };

                // Offset by child's anchor point
                let x = target_x - child_size.width * self.anchor.x;
                let y = target_y - child_size.height * self.anchor.y;

                Rect::new(Point::new(x, y), child_size)
            })
            .collect()
    }
}

// ============================================================================
// PositionedChild<V> - View wrapper with position info
// ============================================================================

/// A view with positioning information, for use in `Absolute` container.
pub struct PositionedChild<V> {
    layout: PositionedLayout,
    content: V,
}

impl<V> fmt::Debug for PositionedChild<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PositionedChild")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl<V: View> View for PositionedChild<V> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(self.layout, (self.content,))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

// ============================================================================
// PositionExt - Extension trait for positioning views
// ============================================================================

/// Extension trait for absolute positioning of views.
///
/// Use these methods inside an `Absolute` container.
pub trait PositionExt: View + Sized {
    /// Position view's center at absolute (x, y) coordinates.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// text("Hello").position(100.0, 50.0)  // center at (100, 50)
    /// ```
    fn position(
        self,
        x: impl IntoComputed<f32>,
        y: impl IntoComputed<f32>,
    ) -> PositionedChild<Self> {
        self.position_anchor(UnitPoint::CENTER, x, y)
    }

    /// Position view's anchor point at absolute coordinates.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// text("Hello").position_anchor(UnitPoint::TOP_LEADING, 10.0, 10.0)
    /// ```
    fn position_anchor(
        self,
        anchor: UnitPoint,
        x: impl IntoComputed<f32>,
        y: impl IntoComputed<f32>,
    ) -> PositionedChild<Self> {
        PositionedChild {
            layout: PositionedLayout {
                anchor,
                target: PositionTarget::Absolute {
                    x: x.into_computed(),
                    y: y.into_computed(),
                },
            },
            content: self,
        }
    }

    /// Position view's center at fractional location in parent.
    ///
    /// The anchor point on the view matches the target position.
    /// e.g., `UnitPoint::CENTER` means view's center at parent's center.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// text("Centered").position_in(UnitPoint::CENTER)
    /// ```
    fn position_in(self, position: UnitPoint) -> PositionedChild<Self> {
        self.position_in_anchor(position, position)
    }

    /// Position view's anchor at fractional location in parent.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Place view's top-left at parent's center
    /// view.position_in_anchor(UnitPoint::TOP_LEADING, UnitPoint::CENTER)
    /// ```
    fn position_in_anchor(self, anchor: UnitPoint, position: UnitPoint) -> PositionedChild<Self> {
        PositionedChild {
            layout: PositionedLayout {
                anchor,
                target: PositionTarget::Fractional {
                    unit: position,
                    offset_x: 0.0_f32.into_computed(),
                    offset_y: 0.0_f32.into_computed(),
                },
            },
            content: self,
        }
    }

    /// Position with fractional location plus offset.
    ///
    /// Great for "16pt from bottom-right corner" patterns.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // FAB at bottom-right with 16pt inset
    /// fab.position_in_offset(
    ///     UnitPoint::BOTTOM_TRAILING,
    ///     UnitPoint::BOTTOM_TRAILING,
    ///     -16.0, -16.0
    /// )
    /// ```
    fn position_in_offset(
        self,
        anchor: UnitPoint,
        position: UnitPoint,
        offset_x: impl IntoComputed<f32>,
        offset_y: impl IntoComputed<f32>,
    ) -> PositionedChild<Self> {
        PositionedChild {
            layout: PositionedLayout {
                anchor,
                target: PositionTarget::Fractional {
                    unit: position,
                    offset_x: offset_x.into_computed(),
                    offset_y: offset_y.into_computed(),
                },
            },
            content: self,
        }
    }
}

impl<V: View + Sized> PositionExt for V {}

// ============================================================================
// Absolute<C> - Container for positioned children
// ============================================================================

/// Container for absolutely positioned children.
///
/// Fills available space and positions each child within.
/// Children can use `PositionExt` methods like `.position()` and `.position_in()`.
///
/// # Example
///
/// ```rust,ignore
/// absolute((
///     Color::gray(),  // Fills automatically (StretchAxis::Both)
///     text("Centered").position_in(UnitPoint::CENTER),
///     fab.position_in_offset(UnitPoint::BOTTOM_TRAILING, UnitPoint::BOTTOM_TRAILING, -16.0, -16.0),
/// ))
/// ```
pub struct Absolute<C> {
    contents: C,
}

impl<C> fmt::Debug for Absolute<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Absolute").finish_non_exhaustive()
    }
}

impl<C: TupleViews> Absolute<C> {
    /// Creates a new `Absolute` container with the given contents.
    #[must_use]
    pub const fn new(contents: C) -> Self {
        Self { contents }
    }
}

impl<C: TupleViews + 'static> View for Absolute<C> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(AbsoluteLayout, self.contents)
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// Creates an `Absolute` container for positioning children.
///
/// The container fills available space and allows children to be
/// positioned at specific coordinates using `PositionExt` methods.
///
/// # Example
///
/// ```rust,ignore
/// absolute((
///     background,
///     text("Hello").position(50.0, 100.0),
///     icon.position_in(UnitPoint::CENTER),
/// ))
/// ```
#[must_use]
pub const fn absolute<C: TupleViews>(contents: C) -> Absolute<C> {
    Absolute::new(contents)
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use super::*;
    use crate::StretchAxis;

    struct MockSubView {
        size: Size,
    }

    impl SubView for MockSubView {
        fn size_that_fits(&self, _proposal: ProposalSize) -> Size {
            self.size
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    #[test]
    fn test_absolute_layout_fills_parent() {
        let layout = AbsoluteLayout;

        // Absolute fills whatever is proposed
        let size = layout.size_that_fits(
            ProposalSize::new(Some(200.0), Some(300.0)),
            &[],
        );
        assert!((size.width - 200.0).abs() < f32::EPSILON);
        assert!((size.height - 300.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_absolute_layout_gives_full_bounds() {
        let layout = AbsoluteLayout;

        let mut child1 = MockSubView {
            size: Size::new(50.0, 50.0),
        };
        let mut child2 = MockSubView {
            size: Size::new(30.0, 30.0),
        };

        let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2];
        let bounds = Rect::new(Point::new(10.0, 20.0), Size::new(200.0, 300.0));

        let rects = layout.place(bounds, &children);

        // All children get full bounds
        assert_eq!(rects.len(), 2);
        assert!((rects[0].x() - 10.0).abs() < f32::EPSILON);
        assert!((rects[0].y() - 20.0).abs() < f32::EPSILON);
        assert!((rects[0].width() - 200.0).abs() < f32::EPSILON);
        assert!((rects[0].height() - 300.0).abs() < f32::EPSILON);
        assert!((rects[1].x() - 10.0).abs() < f32::EPSILON);
        assert!((rects[1].y() - 20.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_positioned_layout_absolute_center() {
        let layout = PositionedLayout {
            anchor: UnitPoint::CENTER,
            target: PositionTarget::Absolute {
                x: 100.0_f32.into_computed(),
                y: 50.0_f32.into_computed(),
            },
        };

        let mut child = MockSubView {
            size: Size::new(40.0, 20.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];
        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(200.0, 200.0));

        let rects = layout.place(bounds, &children);

        // Child's center at (100, 50), so origin is (100 - 20, 50 - 10) = (80, 40)
        assert!((rects[0].x() - 80.0).abs() < f32::EPSILON);
        assert!((rects[0].y() - 40.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_positioned_layout_fractional_center() {
        let layout = PositionedLayout {
            anchor: UnitPoint::CENTER,
            target: PositionTarget::Fractional {
                unit: UnitPoint::CENTER,
                offset_x: 0.0_f32.into_computed(),
                offset_y: 0.0_f32.into_computed(),
            },
        };

        let mut child = MockSubView {
            size: Size::new(40.0, 20.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];
        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(200.0, 100.0));

        let rects = layout.place(bounds, &children);

        // Target is center of parent (100, 50), child center there
        // Origin = (100 - 20, 50 - 10) = (80, 40)
        assert!((rects[0].x() - 80.0).abs() < f32::EPSILON);
        assert!((rects[0].y() - 40.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_positioned_layout_fractional_with_offset() {
        let layout = PositionedLayout {
            anchor: UnitPoint::BOTTOM_TRAILING,
            target: PositionTarget::Fractional {
                unit: UnitPoint::BOTTOM_TRAILING,
                offset_x: (-16.0).into_computed(),
                offset_y: (-16.0).into_computed(),
            },
        };

        let mut child = MockSubView {
            size: Size::new(50.0, 50.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];
        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(200.0, 200.0));

        let rects = layout.place(bounds, &children);

        // Target = bottom-right (200, 200) + offset (-16, -16) = (184, 184)
        // Child's bottom-right at target, so origin = (184 - 50, 184 - 50) = (134, 134)
        assert!((rects[0].x() - 134.0).abs() < f32::EPSILON);
        assert!((rects[0].y() - 134.0).abs() < f32::EPSILON);
    }
}
