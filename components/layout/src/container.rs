//! View wrapper that lets arbitrary [`Layout`] implementations
//! participate in the `WaterUI` view tree.

use core::fmt::Debug;

use alloc::{boxed::Box, vec::Vec};
use waterui_core::{
    AnyView, Native, NativeView, View,
    view::TupleViews,
    views::{AnyViews, Views, ViewsExt},
};

use crate::{Layout, StretchAxis};

/// A view wrapper that executes an arbitrary [`Layout`]
/// implementation.
pub struct FixedContainer {
    layout: Box<dyn Layout>,
    contents: Vec<AnyView>,
}

impl Debug for FixedContainer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Container")
            .field("layout", &"Box<dyn Layout>")
            .field("contents", &self.contents)
            .finish()
    }
}

impl FixedContainer {
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

impl NativeView for FixedContainer {
    fn stretch_axis(&self) -> StretchAxis {
        self.layout.stretch_axis()
    }
}

impl View for FixedContainer {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Native(self)
    }
}

/// A view wrapper that executes an arbitrary [`Layout`] implementation
/// with reconstructable views, which can support lazy layouting.
#[derive(Debug)]
pub struct Container {
    layout: Box<dyn Layout>,
    contents: AnyViews<AnyView>,
}

impl Container {
    /// Wraps the supplied layout object and views into a container view.
    pub fn new<V: View>(
        layout: impl Layout + 'static,
        contents: impl Views<View = V> + 'static,
    ) -> Self {
        Self {
            layout: Box::new(layout),
            contents: AnyViews::new(contents.map(|v| AnyView::new(v))),
        }
    }
    /// Returns the boxed layout object together with the collected child views.
    #[must_use]
    pub fn into_inner(self) -> (Box<dyn Layout>, AnyViews<AnyView>) {
        (self.layout, self.contents)
    }
}

impl NativeView for Container {
    fn stretch_axis(&self) -> StretchAxis {
        self.layout.stretch_axis()
    }
}

impl View for Container {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Native(self)
    }
}
