use alloc::{boxed::Box, vec::Vec};
use waterui_core::{AnyView, raw_view, view::TupleViews};

use crate::Layout;

pub struct Container {
    layout: Box<dyn Layout>,
    contents: Vec<AnyView>,
}

impl Container {
    pub fn new(layout: impl Layout + 'static, contents: impl TupleViews) -> Self {
        Self {
            layout: Box::new(layout),
            contents: contents.into_views(),
        }
    }

    pub fn into_inner(self) -> (Box<dyn Layout>, Vec<AnyView>) {
        (self.layout, self.contents)
    }
}

raw_view!(Container); // Underhood, the renderer will use `layout` trait object to layout its children

#[macro_export]
macro_rules! container {
    ($name:ident,$layout:ty) => {
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
