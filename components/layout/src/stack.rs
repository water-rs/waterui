use alloc::vec::Vec;
use waterui_core::AnyView;
use waterui_core::view::TupleViews;
use waterui_core::{View, raw_view};

#[derive(Debug)]
#[must_use]
/// A stack of views that arranges them in a specific layout mode.
pub struct Stack {
    /// The contents of the stack, which can be any views.
    pub contents: Vec<AnyView>,
    /// The stacking mode that determines how the views are arranged.
    pub mode: StackMode,
}

#[derive(Debug, Default)]
#[repr(C)]
/// Defines the stacking mode for the `Stack` component.
pub enum StackMode {
    /// Stacks views vertically, one on top of the other.
    #[default]
    Vertical,
    /// Stacks views horizontally, side by side.
    Horizonal,
    /// Stacks views in layers, allowing overlapping content.
    Layered,
}

impl Stack {
    /// Creates a new `Stack` with the specified contents and stacking mode.
    pub fn new(contents: impl TupleViews, mode: StackMode) -> Self {
        Self {
            contents: contents.into_views(),
            mode,
        }
    }
    /// Creates a new `Stack` with the specified contents and stacking mode.
    pub fn vertical(contents: impl TupleViews) -> Self {
        Self::new(contents, StackMode::Vertical)
    }
    /// Creates a new `Stack` with the specified contents and stacking mode.
    pub fn horizonal(contents: impl TupleViews) -> Self {
        Self::new(contents, StackMode::Horizonal)
    }
    /// Creates a new `Stack` with the specified contents and stacking mode.
    pub fn layered(contents: impl TupleViews) -> Self {
        Self::new(contents, StackMode::Layered)
    }
}

raw_view!(Stack);

/// Provides convenient functions to create stacks with different orientations.
pub fn vstack(contents: impl TupleViews) -> VStack {
    VStack::new(contents)
}

/// Creates a horizontal stack of views.
pub fn hstack(contents: impl TupleViews) -> HStack {
    HStack::new(contents)
}

/// Creates a layered stack of views, allowing overlapping content.
pub fn zstack(contents: impl TupleViews) -> ZStack {
    ZStack::new(contents)
}

macro_rules! impl_stack {
    ($name:ident,$mode:ident) => {
        #[derive(Debug)]
        /// A stack of views that arranges them in a specific layout mode.
        pub struct $name(Vec<AnyView>);

        impl $name {
            /// Creates a new stack with the specified contents.
            pub fn new(contents: impl TupleViews) -> Self {
                Self(contents.into_views())
            }
        }

        impl<V: View> FromIterator<V> for $name {
            fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
                Self::new(
                    iter.into_iter()
                        .map(|content| AnyView::new(content))
                        .collect::<Vec<_>>(),
                )
            }
        }

        impl View for $name {
            fn body(self, _env: &waterui_core::Environment) -> impl View {
                Stack::$mode(self.0)
            }
        }
    };
}

impl_stack!(VStack, vertical);
impl_stack!(HStack, horizonal);
impl_stack!(ZStack, layered);
