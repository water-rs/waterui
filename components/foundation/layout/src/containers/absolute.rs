//! Absolute positioning layout for `WaterUI`.
//!
//! This module provides an `Absolute` container that fills available space
//! and allows children to be positioned at specific coordinates within.
//!
//! # Example
//!
//! ```rust,ignore
//! use waterui_layout::{absolute, PinConstraints, UnitPoint, PositionExt};
//!
//! absolute((
//!     Color::gray(),  // Fills container
//!     text("Center").position_in(UnitPoint::CENTER),
//!     badge.position_in_offset(
//!         UnitPoint::TOP_TRAILING,
//!         UnitPoint::TOP_TRAILING,
//!         -8.0, 8.0
//!     ),
//!     panel.pin(PinConstraints::all(16.0)),
//! ))
//! ```

use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use nami::{Computed, Signal, SignalExt, watcher::BoxWatcherGuard};
use waterui_core::{IntoSignalF32, View, layout::LayoutInvalidationCallback, view::TupleViews};

use crate::{
    Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView, UnitPoint,
    container::FixedContainer,
};

// ============================================================================
// AbsoluteLayout - Fills parent, gives all children full bounds
// ============================================================================

/// Layout that fills parent and gives each child full bounds.
///
/// Children are responsible for their own positioning (via `PositionedLayout`).
#[derive(Debug, Clone, Default)]
pub struct AbsoluteLayout;

impl Layout for AbsoluteLayout {
    fn stretch_axis(&self, _children: &[StretchAxis]) -> StretchAxis {
        StretchAxis::Both
    }

    fn size_that_fits(&self, proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
        // Absolute fills offered space. In unconstrained contexts we use 0
        // to avoid propagating infinite dimensions to native backends.
        Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
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
    /// Edge-based pinning with optional explicit size.
    Pinned(PinConstraints),
}

/// Edge-based constraints for absolute positioning.
///
/// This supports common pinning patterns:
/// - `leading + trailing` computes width from parent bounds
/// - `top + bottom` computes height from parent bounds
/// - explicit `width` / `height` override computed dimensions
#[derive(Debug, Clone, Default)]
pub struct PinConstraints {
    leading: Option<Computed<f32>>,
    trailing: Option<Computed<f32>>,
    top: Option<Computed<f32>>,
    bottom: Option<Computed<f32>>,
    width: Option<Computed<f32>>,
    height: Option<Computed<f32>>,
}

impl PinConstraints {
    /// Creates empty pin constraints.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            leading: None,
            trailing: None,
            top: None,
            bottom: None,
            width: None,
            height: None,
        }
    }

    /// Pins all four edges by the same inset.
    #[must_use]
    pub fn all(inset: impl IntoSignalF32) -> Self {
        let inset = inset.into_signal_f32().computed();
        Self {
            leading: Some(inset.clone()),
            trailing: Some(inset.clone()),
            top: Some(inset.clone()),
            bottom: Some(inset),
            width: None,
            height: None,
        }
    }

    /// Pins leading edge.
    #[must_use]
    pub fn leading(mut self, value: impl IntoSignalF32) -> Self {
        self.leading = Some(value.into_signal_f32().computed());
        self
    }

    /// Pins trailing edge.
    #[must_use]
    pub fn trailing(mut self, value: impl IntoSignalF32) -> Self {
        self.trailing = Some(value.into_signal_f32().computed());
        self
    }

    /// Pins top edge.
    #[must_use]
    pub fn top(mut self, value: impl IntoSignalF32) -> Self {
        self.top = Some(value.into_signal_f32().computed());
        self
    }

    /// Pins bottom edge.
    #[must_use]
    pub fn bottom(mut self, value: impl IntoSignalF32) -> Self {
        self.bottom = Some(value.into_signal_f32().computed());
        self
    }

    /// Sets explicit width.
    #[must_use]
    pub fn width(mut self, value: impl IntoSignalF32) -> Self {
        self.width = Some(value.into_signal_f32().computed());
        self
    }

    /// Sets explicit height.
    #[must_use]
    pub fn height(mut self, value: impl IntoSignalF32) -> Self {
        self.height = Some(value.into_signal_f32().computed());
        self
    }
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
    fn stretch_axis(&self, _children: &[StretchAxis]) -> StretchAxis {
        StretchAxis::Both
    }

    fn size_that_fits(&self, proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
        Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        )
    }

    #[allow(clippy::too_many_lines)]
    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        let child_proposal = ProposalSize::new(
            bounds
                .width()
                .is_finite()
                .then_some(bounds.width().max(0.0)),
            bounds
                .height()
                .is_finite()
                .then_some(bounds.height().max(0.0)),
        );

        children
            .iter()
            .map(|child| {
                let intrinsic = child.measure(child_proposal).size;

                match &self.target {
                    PositionTarget::Absolute { x, y } => {
                        let child_size = sanitize_size(intrinsic, bounds);
                        let target_x = bounds.x() + x.get();
                        let target_y = bounds.y() + y.get();

                        let x = target_x - child_size.width * self.anchor.x;
                        let y = target_y - child_size.height * self.anchor.y;

                        Rect::new(
                            Point::new(
                                if x.is_finite() { x } else { bounds.x() },
                                if y.is_finite() { y } else { bounds.y() },
                            ),
                            child_size,
                        )
                    }
                    PositionTarget::Fractional {
                        unit,
                        offset_x,
                        offset_y,
                    } => {
                        let child_size = sanitize_size(intrinsic, bounds);
                        let target_x = bounds.x() + bounds.width() * unit.x + offset_x.get();
                        let target_y = bounds.y() + bounds.height() * unit.y + offset_y.get();

                        let x = target_x - child_size.width * self.anchor.x;
                        let y = target_y - child_size.height * self.anchor.y;

                        Rect::new(
                            Point::new(
                                if x.is_finite() { x } else { bounds.x() },
                                if y.is_finite() { y } else { bounds.y() },
                            ),
                            child_size,
                        )
                    }
                    PositionTarget::Pinned(pinned) => {
                        let leading = pinned.leading.as_ref().map(waterui_core::Signal::get);
                        let trailing = pinned.trailing.as_ref().map(waterui_core::Signal::get);
                        let top = pinned.top.as_ref().map(waterui_core::Signal::get);
                        let bottom = pinned.bottom.as_ref().map(waterui_core::Signal::get);
                        let explicit_width = pinned.width.as_ref().map(waterui_core::Signal::get);
                        let explicit_height = pinned.height.as_ref().map(waterui_core::Signal::get);

                        let mut width = explicit_width.unwrap_or_else(|| {
                            if let (Some(leading), Some(trailing)) = (leading, trailing) {
                                bounds.width() - leading - trailing
                            } else {
                                intrinsic.width
                            }
                        });
                        let mut height = explicit_height.unwrap_or_else(|| {
                            if let (Some(top), Some(bottom)) = (top, bottom) {
                                bounds.height() - top - bottom
                            } else {
                                intrinsic.height
                            }
                        });

                        if !width.is_finite() {
                            width = bounds.width();
                        }
                        if !height.is_finite() {
                            height = bounds.height();
                        }
                        width = width.max(0.0);
                        height = height.max(0.0);

                        let measured = child
                            .measure(ProposalSize::new(Some(width), Some(height)))
                            .size;
                        if explicit_width.is_none() && !(leading.is_some() && trailing.is_some()) {
                            width = sanitize_axis(measured.width, width);
                        }
                        if explicit_height.is_none() && !(top.is_some() && bottom.is_some()) {
                            height = sanitize_axis(measured.height, height);
                        }

                        let x = leading.map_or_else(
                            || {
                                trailing.map_or_else(
                                    || bounds.x(),
                                    |trailing| bounds.max_x() - trailing - width,
                                )
                            },
                            |leading| bounds.x() + leading,
                        );

                        let y = top.map_or_else(
                            || {
                                bottom.map_or_else(
                                    || bounds.y(),
                                    |bottom| bounds.max_y() - bottom - height,
                                )
                            },
                            |top| bounds.y() + top,
                        );

                        Rect::new(
                            Point::new(
                                if x.is_finite() { x } else { bounds.x() },
                                if y.is_finite() { y } else { bounds.y() },
                            ),
                            Size::new(width, height),
                        )
                    }
                }
            })
            .collect()
    }

    fn watch_invalidation(&self, invalidate: LayoutInvalidationCallback) -> Vec<BoxWatcherGuard> {
        let signals: Vec<&Computed<f32>> = match &self.target {
            PositionTarget::Absolute { x, y } => vec![x, y],
            PositionTarget::Fractional {
                offset_x, offset_y, ..
            } => vec![offset_x, offset_y],
            PositionTarget::Pinned(pinned) => [
                pinned.leading.as_ref(),
                pinned.trailing.as_ref(),
                pinned.top.as_ref(),
                pinned.bottom.as_ref(),
                pinned.width.as_ref(),
                pinned.height.as_ref(),
            ]
            .into_iter()
            .flatten()
            .collect(),
        };
        let mut identities = BTreeSet::new();
        let mut guards = Vec::new();
        for signal in signals {
            if signal
                .identity()
                .is_some_and(|identity| !identities.insert(identity))
            {
                continue;
            }
            let invalidate = invalidate.clone();
            guards.push(signal.watch(move |_| invalidate()));
        }
        guards
    }
}

const fn sanitize_size(intrinsic: Size, bounds: Rect) -> Size {
    let width = if intrinsic.width.is_finite() {
        intrinsic.width.max(0.0)
    } else {
        bounds.width().max(0.0)
    };
    let height = if intrinsic.height.is_finite() {
        intrinsic.height.max(0.0)
    } else {
        bounds.height().max(0.0)
    };
    Size::new(width, height)
}

const fn sanitize_axis(measured: f32, fallback: f32) -> f32 {
    if measured.is_finite() {
        measured.max(0.0)
    } else {
        fallback.max(0.0)
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
    fn position(self, x: impl IntoSignalF32, y: impl IntoSignalF32) -> PositionedChild<Self> {
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
        x: impl IntoSignalF32,
        y: impl IntoSignalF32,
    ) -> PositionedChild<Self> {
        PositionedChild {
            layout: PositionedLayout {
                anchor,
                target: PositionTarget::Absolute {
                    x: x.into_signal_f32().computed(),
                    y: y.into_signal_f32().computed(),
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
                    offset_x: 0.0_f32.into_signal_f32().computed(),
                    offset_y: 0.0_f32.into_signal_f32().computed(),
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
        offset_x: impl IntoSignalF32,
        offset_y: impl IntoSignalF32,
    ) -> PositionedChild<Self> {
        PositionedChild {
            layout: PositionedLayout {
                anchor,
                target: PositionTarget::Fractional {
                    unit: position,
                    offset_x: offset_x.into_signal_f32().computed(),
                    offset_y: offset_y.into_signal_f32().computed(),
                },
            },
            content: self,
        }
    }

    /// Position using edge pins and optional explicit size.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Fill parent with 12pt inset
    /// background.pin(PinConstraints::all(12.0));
    ///
    /// // Bottom trailing fixed-size badge
    /// badge.pin(
    ///     PinConstraints::new()
    ///         .trailing(12.0)
    ///         .bottom(12.0)
    ///         .width(28.0)
    ///         .height(28.0)
    /// );
    /// ```
    fn pin(self, constraints: PinConstraints) -> PositionedChild<Self> {
        PositionedChild {
            layout: PositionedLayout {
                anchor: UnitPoint::TOP_LEADING,
                target: PositionTarget::Pinned(constraints),
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
/// Children can use `PositionExt` methods like `.position()`, `.position_in()`,
/// and `.pin()`.
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
    use super::*;
    use crate::StretchAxis;
    use crate::ViewDimensions;
    use alloc::vec;
    use core::cell::Cell;
    use waterui_core::MainThreadBound;

    struct MockSubView {
        size: Size,
        stretch_axis: StretchAxis,
    }

    impl SubView for MockSubView {
        fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(self.size)
        }
        fn stretch_axis(&self) -> StretchAxis {
            self.stretch_axis
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    struct ProposalAwareView {
        intrinsic: Size,
        constrained: Size,
        last_width: MainThreadBound<Cell<Option<f32>>>,
        last_height: MainThreadBound<Cell<Option<f32>>>,
    }

    impl ProposalAwareView {
        fn new(intrinsic: Size, constrained: Size) -> Self {
            Self {
                intrinsic,
                constrained,
                last_width: MainThreadBound::new(Cell::new(None)),
                last_height: MainThreadBound::new(Cell::new(None)),
            }
        }
    }

    impl SubView for ProposalAwareView {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            self.last_width.set(proposal.width);
            self.last_height.set(proposal.height);
            ViewDimensions::new(if proposal.width.is_some() || proposal.height.is_some() {
                self.constrained
            } else {
                self.intrinsic
            })
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::Both
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    #[test]
    fn test_absolute_layout_uses_finite_size_for_unspecified_proposal() {
        let layout = AbsoluteLayout;
        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &[]);

        assert!(size.width.is_finite());
        assert!(size.height.is_finite());
        assert_eq!(size.width, 0.0);
        assert_eq!(size.height, 0.0);
    }

    #[test]
    fn test_absolute_layout_respects_parent_constraints() {
        let layout = AbsoluteLayout;
        let size = layout.size_that_fits(ProposalSize::new(Some(200.0), Some(300.0)), &[]);
        assert_eq!(size.width, 200.0);
        assert_eq!(size.height, 300.0);
    }

    #[test]
    fn test_absolute_layout_gives_full_bounds_to_each_child() {
        let layout = AbsoluteLayout;
        let mut child1 = MockSubView {
            size: Size::new(50.0, 50.0),
            stretch_axis: StretchAxis::None,
        };
        let mut child2 = MockSubView {
            size: Size::new(30.0, 30.0),
            stretch_axis: StretchAxis::None,
        };
        let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2];
        let bounds = Rect::new(Point::new(10.0, 20.0), Size::new(200.0, 300.0));
        let rects = layout.place(bounds, &children);

        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0], bounds);
        assert_eq!(rects[1], bounds);
    }

    #[test]
    fn test_positioned_layout_uses_finite_size_for_unspecified_proposal() {
        let layout = PositionedLayout {
            anchor: UnitPoint::CENTER,
            target: PositionTarget::Absolute {
                x: 10.0_f32.into_signal_f32().computed(),
                y: 20.0_f32.into_signal_f32().computed(),
            },
        };
        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &[]);

        assert!(size.width.is_finite());
        assert!(size.height.is_finite());
        assert_eq!(size.width, 0.0);
        assert_eq!(size.height, 0.0);
    }

    #[test]
    fn test_positioned_layout_measures_child_with_parent_bounds_proposal() {
        let layout = PositionedLayout {
            anchor: UnitPoint::CENTER,
            target: PositionTarget::Fractional {
                unit: UnitPoint::CENTER,
                offset_x: 0.0_f32.into_signal_f32().computed(),
                offset_y: 0.0_f32.into_signal_f32().computed(),
            },
        };

        let child = ProposalAwareView::new(Size::new(10.0, 10.0), Size::new(200.0, 100.0));
        let children: Vec<&dyn SubView> = vec![&child];
        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(200.0, 100.0));

        let rects = layout.place(bounds, &children);

        assert_eq!(child.last_width.get(), Some(200.0));
        assert_eq!(child.last_height.get(), Some(100.0));
        assert_eq!(rects[0].x(), 0.0);
        assert_eq!(rects[0].y(), 0.0);
        assert_eq!(rects[0].width(), 200.0);
        assert_eq!(rects[0].height(), 100.0);
    }

    #[test]
    fn test_positioned_layout_absolute_anchor_math() {
        let layout = PositionedLayout {
            anchor: UnitPoint::BOTTOM_TRAILING,
            target: PositionTarget::Absolute {
                x: 50.0_f32.into_signal_f32().computed(),
                y: 40.0_f32.into_signal_f32().computed(),
            },
        };

        let mut child = MockSubView {
            size: Size::new(20.0, 10.0),
            stretch_axis: StretchAxis::None,
        };
        let children: Vec<&dyn SubView> = vec![&mut child];
        let bounds = Rect::new(Point::new(10.0, 20.0), Size::new(200.0, 100.0));

        let rects = layout.place(bounds, &children);

        // Target at (bounds.x + 50, bounds.y + 40) = (60, 60),
        // bottom-trailing anchor offsets by (20, 10).
        assert_eq!(rects[0].x(), 40.0);
        assert_eq!(rects[0].y(), 50.0);
        assert_eq!(rects[0].width(), 20.0);
        assert_eq!(rects[0].height(), 10.0);
    }

    #[test]
    fn test_positioned_layout_fractional_with_offset() {
        let layout = PositionedLayout {
            anchor: UnitPoint::BOTTOM_TRAILING,
            target: PositionTarget::Fractional {
                unit: UnitPoint::BOTTOM_TRAILING,
                offset_x: (-16.0).into_signal_f32().computed(),
                offset_y: (-16.0).into_signal_f32().computed(),
            },
        };

        let mut child = MockSubView {
            size: Size::new(50.0, 50.0),
            stretch_axis: StretchAxis::None,
        };
        let children: Vec<&dyn SubView> = vec![&mut child];
        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(200.0, 200.0));

        let rects = layout.place(bounds, &children);

        // Target = bottom-right (200, 200) + offset (-16, -16) = (184, 184)
        // Child's bottom-right at target, so origin = (184 - 50, 184 - 50) = (134, 134)
        assert!((rects[0].x() - 134.0).abs() < f32::EPSILON);
        assert!((rects[0].y() - 134.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_pin_constraints_fill_with_insets() {
        let layout = PositionedLayout {
            anchor: UnitPoint::TOP_LEADING,
            target: PositionTarget::Pinned(PinConstraints::all(12.0)),
        };

        let mut child = MockSubView {
            size: Size::new(20.0, 20.0),
            stretch_axis: StretchAxis::None,
        };
        let children: Vec<&dyn SubView> = vec![&mut child];
        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(200.0, 100.0));

        let rects = layout.place(bounds, &children);

        assert_eq!(rects[0].x(), 12.0);
        assert_eq!(rects[0].y(), 12.0);
        assert_eq!(rects[0].width(), 176.0);
        assert_eq!(rects[0].height(), 76.0);
    }

    #[test]
    fn test_pin_constraints_bottom_trailing_fixed_size() {
        let layout = PositionedLayout {
            anchor: UnitPoint::TOP_LEADING,
            target: PositionTarget::Pinned(
                PinConstraints::new()
                    .trailing(16.0)
                    .bottom(10.0)
                    .width(40.0)
                    .height(20.0),
            ),
        };

        let mut child = MockSubView {
            size: Size::new(8.0, 8.0),
            stretch_axis: StretchAxis::None,
        };
        let children: Vec<&dyn SubView> = vec![&mut child];
        let bounds = Rect::new(Point::new(5.0, 10.0), Size::new(200.0, 100.0));

        let rects = layout.place(bounds, &children);

        assert_eq!(rects[0].x(), 149.0);
        assert_eq!(rects[0].y(), 80.0);
        assert_eq!(rects[0].width(), 40.0);
        assert_eq!(rects[0].height(), 20.0);
    }

    #[test]
    fn test_pin_constraints_clamp_negative_size() {
        let layout = PositionedLayout {
            anchor: UnitPoint::TOP_LEADING,
            target: PositionTarget::Pinned(
                PinConstraints::new()
                    .leading(80.0)
                    .trailing(80.0)
                    .top(40.0)
                    .bottom(40.0),
            ),
        };

        let mut child = MockSubView {
            size: Size::new(30.0, 30.0),
            stretch_axis: StretchAxis::None,
        };
        let children: Vec<&dyn SubView> = vec![&mut child];
        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 60.0));

        let rects = layout.place(bounds, &children);

        assert_eq!(rects[0].x(), 80.0);
        assert_eq!(rects[0].y(), 40.0);
        assert_eq!(rects[0].width(), 0.0);
        assert_eq!(rects[0].height(), 0.0);
    }

    #[test]
    fn test_pin_constraints_remeasure_after_width_resolution() {
        let layout = PositionedLayout {
            anchor: UnitPoint::TOP_LEADING,
            target: PositionTarget::Pinned(PinConstraints::new().leading(10.0).trailing(10.0)),
        };

        let child = ProposalAwareView::new(Size::new(120.0, 20.0), Size::new(100.0, 40.0));
        let children: Vec<&dyn SubView> = vec![&child];
        let bounds = Rect::new(Point::zero(), Size::new(120.0, 200.0));

        let rects = layout.place(bounds, &children);

        assert_eq!(rects[0].x(), 10.0);
        assert_eq!(rects[0].width(), 100.0);
        assert_eq!(rects[0].height(), 40.0);
        assert_eq!(child.last_width.get(), Some(100.0));
    }
}
