//! Stack-based layout primitives.
//!
//! The submodules implement horizontal, vertical, and overlay stacks. These
//! views arrange child content according to alignments and spacing and are the
//! backbone of most declarative layouts in `WaterUI`.
//!
//! ![Stack](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/stack.svg)

/// Emits `for_each` on a stack wrapper around `ForEach<C, F, V>`.
///
/// Reused by HStack/VStack/ZStack so the three families share a single
/// implementation without exposing a trait users would have to import.
/// The macro is private to this crate (no `#[macro_export]`) and is
/// invoked at the call site in each stack module.
macro_rules! impl_stack_for_each {
    ($Stack:ident, $Layout:ident, $virtualize_axes:expr) => {
        impl<C, F, V> $Stack<ForEach<C, F, V>>
        where
            C: Collection + Clone,
            C::Item: Clone,
            F: 'static + Fn(C::Item) -> V,
            V: View,
        {
            /// Creates the stack by iterating over a collection and generating
            /// views.
            ///
            /// Rows take their identity from the collection: an array or static
            /// slice identifies each row by its index, and a reactive `List` of
            /// `Identifiable` items by each item's id. Inside a co-axial
            /// [`ScrollView`](crate::scroll::ScrollView), rows virtualize along
            /// the stack's axis unless [`eager`](Self::eager) is called.
            pub fn for_each(collection: C, generator: F) -> Self {
                Self {
                    layout: $Layout::default(),
                    contents: ForEach::new(collection, generator),
                    virtualize_axes: $virtualize_axes,
                }
            }

            /// Opts out of viewport virtualization and materializes every item
            /// eagerly even when enclosed by a co-axial
            /// [`ScrollView`](crate::scroll::ScrollView).
            #[must_use]
            pub const fn eager(mut self) -> Self {
                self.virtualize_axes = None;
                self
            }
        }
    };
    ($Stack:ident, $Layout:ident) => {
        impl<C, F, V> $Stack<ForEach<C, F, V>>
        where
            C: Collection + Clone,
            C::Item: Clone,
            F: 'static + Fn(C::Item) -> V,
            V: View,
        {
            /// Creates the stack by iterating over a collection and generating
            /// views.
            pub fn for_each(collection: C, generator: F) -> Self {
                Self {
                    layout: $Layout::default(),
                    contents: ForEach::new(collection, generator),
                }
            }
        }
    };
}

pub(crate) use impl_stack_for_each;

pub(crate) mod distribute;
mod vstack;
pub use vstack::*;
mod hstack;
pub use hstack::*;
mod zstack;
pub use zstack::*;

pub use waterui_core::layout::{Alignment, HorizontalAlignment, VerticalAlignment};

use crate::Layout;
use crate::scroll::Axis as ScrollAxis;
use nami::Computed;
use waterui_core::layout::StretchAxis;

/// The stretch axis a stack of `children` claims, along `main`.
///
/// A stack claims nothing of its own: a row of labels stays content-sized, as
/// `SwiftUI`'s does. What it cannot do is stay silent about a child that wants
/// to fill. A stack's parent hands it the size it reports, and only then does
/// the stack distribute that size among its children — so a row that holds a
/// `Color`, a `Spacer` or a filling frame and reports "content-sized" is handed
/// its own intrinsic size, and the greedy child fills a box of nothing. The
/// space a child asks for has to be asked for again by every container between
/// it and whoever owns the space.
///
/// `MainAxis` and `CrossAxis` resolve against `main`, the same way each stack's
/// own child-measurement helpers resolve them.
pub(crate) fn stack_stretch_axis(main: Axis, children: &[StretchAxis]) -> StretchAxis {
    let mut fills_main = false;
    let mut fills_cross = false;
    for child in children {
        match child {
            StretchAxis::None => {}
            StretchAxis::Both => {
                fills_main = true;
                fills_cross = true;
            }
            StretchAxis::MainAxis => fills_main = true,
            StretchAxis::CrossAxis => fills_cross = true,
            StretchAxis::Horizontal => {
                if main.is_horizontal() {
                    fills_main = true;
                } else {
                    fills_cross = true;
                }
            }
            StretchAxis::Vertical => {
                if main.is_vertical() {
                    fills_main = true;
                } else {
                    fills_cross = true;
                }
            }
        }
    }
    let (horizontal, vertical) = if main.is_horizontal() {
        (fills_main, fills_cross)
    } else {
        (fills_cross, fills_main)
    };
    match (horizontal, vertical) {
        (true, true) => StretchAxis::Both,
        (true, false) => StretchAxis::Horizontal,
        (false, true) => StretchAxis::Vertical,
        (false, false) => StretchAxis::None,
    }
}

/// The union of child stretch claims for a container that has no main axis of
/// its own (`ZStack`, `Grid`). Axis-relative claims resolve the way the
/// container resolves them for its children: `MainAxis` wants whatever the
/// main axis offers, and with no main axis there is nothing to claim — a
/// spacer claims nothing in a `ZStack` — while `CrossAxis` resolves to its
/// default orientation, filling horizontally.
pub(crate) fn axisless_stretch_union(children: &[StretchAxis]) -> StretchAxis {
    let mut fills_h = false;
    let mut fills_v = false;
    for child in children {
        match child {
            StretchAxis::None | StretchAxis::MainAxis => {}
            StretchAxis::Both => {
                fills_h = true;
                fills_v = true;
            }
            StretchAxis::Horizontal | StretchAxis::CrossAxis => fills_h = true,
            StretchAxis::Vertical => fills_v = true,
        }
    }
    match (fills_h, fills_v) {
        (true, true) => StretchAxis::Both,
        (true, false) => StretchAxis::Horizontal,
        (false, true) => StretchAxis::Vertical,
        (false, false) => StretchAxis::None,
    }
}

/// Defines the axis of a stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Axis {
    /// Horizontal axis is the x-axis (`HStack`)
    Horizontal,
    /// Vertical axis is the y-axis (`VStack`)
    Vertical,
}

impl Axis {
    /// Returns true if this axis is horizontal.
    #[must_use]
    pub const fn is_horizontal(&self) -> bool {
        matches!(self, Self::Horizontal)
    }

    /// Returns true if this axis is vertical.
    #[must_use]
    pub const fn is_vertical(&self) -> bool {
        matches!(self, Self::Vertical)
    }
}

/// The axis a [`LazyContainer`](crate::container::LazyContainer) can be
/// virtualized along, together with the spacing and cross-axis alignment the
/// stack was configured with.
///
/// Only the two stacks scroll in one direction with a well-defined item order,
/// which is what viewport virtualization needs; every other layout — a grid, a
/// `ZStack`, an absolute overlay — lays out its whole membership and returns
/// `None` here.
#[derive(Debug, Clone)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed: a lazy stack runs vertically or horizontally"
)]
pub enum LazyStackAxis {
    /// Items run top to bottom, aligned horizontally.
    Vertical {
        /// Gap between consecutive items.
        spacing: Computed<f32>,
        /// How items line up across the axis.
        alignment: HorizontalAlignment,
    },
    /// Items run leading to trailing, aligned vertically.
    Horizontal {
        /// Gap between consecutive items.
        spacing: Computed<f32>,
        /// How items line up across the axis.
        alignment: VerticalAlignment,
    },
}

/// Resolves a layout to the axis a lazy container can virtualize along.
///
/// Every renderer that virtualizes a lazy container asks this, so the answer is
/// the same everywhere. Deriving it per backend is how GTK ended up reading
/// `Layout::stretch_axis` — a different question entirely, and one whose answer
/// changed when the stacks became content-sized, silently laying every lazy
/// horizontal stack out vertically.
#[must_use]
pub fn lazy_stack_axis(layout: &dyn Layout) -> Option<LazyStackAxis> {
    let layout = layout as &dyn core::any::Any;
    if let Some(vstack) = layout.downcast_ref::<VStackLayout>() {
        return Some(LazyStackAxis::Vertical {
            spacing: vstack.spacing.clone(),
            alignment: vstack.alignment,
        });
    }
    if let Some(hstack) = layout.downcast_ref::<HStackLayout>() {
        return Some(LazyStackAxis::Horizontal {
            spacing: hstack.spacing.clone(),
            alignment: hstack.alignment,
        });
    }
    None
}

/// Resolves a layout to the virtualized stack axis only when `virtualize_axes`
/// permits virtualization along that stack's axis.
#[must_use]
pub fn lazy_stack_axis_for(
    layout: &dyn Layout,
    virtualize_axes: Option<ScrollAxis>,
) -> Option<LazyStackAxis> {
    let virtualize_axes = virtualize_axes?;
    let axis = lazy_stack_axis(layout)?;
    let allowed = match axis {
        LazyStackAxis::Vertical { .. } => virtualize_axes.allows_vertical(),
        LazyStackAxis::Horizontal { .. } => virtualize_axes.allows_horizontal(),
    };
    allowed.then_some(axis)
}

/// Projects a sampled lazy-stack item's stretch claim onto the stack's cross
/// axis so an enclosing [`ScrollView`](crate::scroll::ScrollView) preserves
/// cross-axis filling without stretching along the scrolling main axis.
#[must_use]
pub const fn lazy_stack_cross_stretch(axis: Axis, item_stretch: StretchAxis) -> StretchAxis {
    match axis {
        Axis::Vertical => {
            if matches!(
                item_stretch,
                StretchAxis::Horizontal | StretchAxis::Both | StretchAxis::CrossAxis
            ) {
                StretchAxis::Horizontal
            } else {
                StretchAxis::None
            }
        }
        Axis::Horizontal => {
            if matches!(
                item_stretch,
                StretchAxis::Vertical | StretchAxis::Both | StretchAxis::CrossAxis
            ) {
                StretchAxis::Vertical
            } else {
                StretchAxis::None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_axis_intersection_matches_shared_axes() {
        assert_eq!(
            ScrollAxis::Vertical.intersect(ScrollAxis::Vertical),
            Some(ScrollAxis::Vertical)
        );
        assert_eq!(
            ScrollAxis::Horizontal.intersect(ScrollAxis::Horizontal),
            Some(ScrollAxis::Horizontal)
        );
        assert_eq!(
            ScrollAxis::All.intersect(ScrollAxis::Vertical),
            Some(ScrollAxis::Vertical)
        );
        assert_eq!(
            ScrollAxis::All.intersect(ScrollAxis::Horizontal),
            Some(ScrollAxis::Horizontal)
        );
        assert_eq!(
            ScrollAxis::All.intersect(ScrollAxis::All),
            Some(ScrollAxis::All)
        );
        assert_eq!(ScrollAxis::Vertical.intersect(ScrollAxis::Horizontal), None);
        assert_eq!(ScrollAxis::Horizontal.intersect(ScrollAxis::Vertical), None);
    }

    #[test]
    fn lazy_stack_axis_for_gates_by_virtualize_axes() {
        let vstack = VStackLayout::default();
        let hstack = HStackLayout::default();
        let zstack = ZStackLayout::default();

        assert!(matches!(
            lazy_stack_axis_for(&vstack, Some(ScrollAxis::Vertical)),
            Some(LazyStackAxis::Vertical { .. })
        ));
        assert!(matches!(
            lazy_stack_axis_for(&vstack, Some(ScrollAxis::All)),
            Some(LazyStackAxis::Vertical { .. })
        ));
        assert!(lazy_stack_axis_for(&vstack, Some(ScrollAxis::Horizontal)).is_none());
        assert!(lazy_stack_axis_for(&vstack, None).is_none());

        assert!(matches!(
            lazy_stack_axis_for(&hstack, Some(ScrollAxis::Horizontal)),
            Some(LazyStackAxis::Horizontal { .. })
        ));
        assert!(matches!(
            lazy_stack_axis_for(&hstack, Some(ScrollAxis::All)),
            Some(LazyStackAxis::Horizontal { .. })
        ));
        assert!(lazy_stack_axis_for(&hstack, Some(ScrollAxis::Vertical)).is_none());
        assert!(lazy_stack_axis_for(&hstack, None).is_none());

        assert!(lazy_stack_axis_for(&zstack, Some(ScrollAxis::All)).is_none());
    }

    #[test]
    fn lazy_stack_cross_stretch_projects_only_cross_axis() {
        assert_eq!(
            lazy_stack_cross_stretch(Axis::Vertical, StretchAxis::Horizontal),
            StretchAxis::Horizontal
        );
        assert_eq!(
            lazy_stack_cross_stretch(Axis::Vertical, StretchAxis::Both),
            StretchAxis::Horizontal
        );
        assert_eq!(
            lazy_stack_cross_stretch(Axis::Vertical, StretchAxis::CrossAxis),
            StretchAxis::Horizontal
        );
        assert_eq!(
            lazy_stack_cross_stretch(Axis::Vertical, StretchAxis::Vertical),
            StretchAxis::None
        );
        assert_eq!(
            lazy_stack_cross_stretch(Axis::Vertical, StretchAxis::MainAxis),
            StretchAxis::None
        );
        assert_eq!(
            lazy_stack_cross_stretch(Axis::Horizontal, StretchAxis::Vertical),
            StretchAxis::Vertical
        );
        assert_eq!(
            lazy_stack_cross_stretch(Axis::Horizontal, StretchAxis::Both),
            StretchAxis::Vertical
        );
        assert_eq!(
            lazy_stack_cross_stretch(Axis::Horizontal, StretchAxis::CrossAxis),
            StretchAxis::Vertical
        );
        assert_eq!(
            lazy_stack_cross_stretch(Axis::Horizontal, StretchAxis::Horizontal),
            StretchAxis::None
        );
    }
}
