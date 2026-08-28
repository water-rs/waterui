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
    ($Stack:ident, $Layout:ident) => {
        impl<C, F, V> $Stack<ForEach<C, F, V>>
        where
            C: Collection,
            C::Item: Identifiable,
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

mod distribute;
mod vstack;
pub use vstack::*;
mod hstack;
pub use hstack::*;
mod zstack;
pub use zstack::*;

pub use waterui_core::layout::{Alignment, HorizontalAlignment, VerticalAlignment};

use crate::Layout;
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
