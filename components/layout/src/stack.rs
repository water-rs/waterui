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

mod vstack;
pub use vstack::*;
mod hstack;
pub use hstack::*;
mod zstack;
pub use zstack::*;

pub use waterui_core::layout::{Alignment, HorizontalAlignment, VerticalAlignment};

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
