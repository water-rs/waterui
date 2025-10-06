//! View wrapper that lets arbitrary [`Layout`] implementations
//! participate in the `WaterUI` view tree.

use core::fmt::Debug;

use alloc::{boxed::Box, vec::Vec};
use waterui_core::{AnyView, raw_view, view::TupleViews};

use crate::Layout;

/// A view wrapper that executes an arbitrary [`Layout`]
/// implementation.
pub struct Container {
    layout: Box<dyn Layout>,
    contents: Vec<AnyView>,
}

impl Debug for Container {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Container")
            .field("layout", &"Box<dyn Layout>")
            .field("contents", &self.contents)
            .finish()
    }
}

impl Container {
    /// Wraps the supplied layout object and tuple of child views into a
    /// container view.
    pub fn new(layout: impl Layout + 'static, contents: impl TupleViews) -> Self {
        Self {
            layout: Box::new(layout),
            contents: contents.into_views(),
        }
    }

    /// Returns the boxed layout object together with the collected child views.
    #[must_use]
    pub fn into_inner(self) -> (Box<dyn Layout>, Vec<AnyView>) {
        (self.layout, self.contents)
    }
}

raw_view!(Container); // Under the hood the renderer drives the layout trait object.

/// Creates a container view type that forwards to [`Container`].
///
/// The macro produces a new view struct with `layout` and `contents` fields and
/// implements [`View`](waterui_core::View) for it by delegating to
/// [`Container::new`]. Callers provide the documentation string so generated
/// types show up in user-facing docs.
///
/// ```no_run
/// use waterui_layout::{container, core::{Layout, ProposalSize, ChildMetadata, Rect, Size}};
///
/// #[derive(Default)]
/// struct ZeroLayout;
///
/// impl Layout for ZeroLayout {
///     fn propose(&mut self, _parent: ProposalSize, _children: &[ChildMetadata]) -> Vec<ProposalSize> {
///         Vec::new()
///     }
///
///     fn size(&mut self, _parent: ProposalSize, _children: &[ChildMetadata]) -> Size {
///         Size::zero()
///     }
///
///     fn place(
///         &mut self,
///         _bound: Rect,
///         _proposal: ProposalSize,
///         _children: &[ChildMetadata],
///     ) -> Vec<Rect> {
///         Vec::new()
///     }
/// }
///
/// container!(MyContainer, ZeroLayout, "Delegates to ZeroLayout during layout.");
/// ```
#[macro_export]
macro_rules! container {
    ($name:ident,$layout:ty,$doc:expr) => {
        #[derive(Debug)]
        #[doc=$doc]
        pub struct $name {
            layout: $layout,
            contents: alloc::vec::Vec<waterui_core::AnyView>,
        }

        impl waterui_core::View for $name {
            fn body(self, _env: &waterui_core::Environment) -> impl waterui_core::View {
                $crate::container::Container::new(self.layout, self.contents)
            }
        }
    };
}
