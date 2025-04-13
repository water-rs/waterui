use alloc::vec::Vec;
use waterui_core::AnyView;
use waterui_core::view::TupleViews;
use waterui_core::{View, raw_view};

#[derive(Debug)]
#[must_use]
pub struct Stack {
    pub contents: Vec<AnyView>,
    pub mode: StackMode,
}

#[derive(Debug, Default)]
#[repr(C)]
pub enum StackMode {
    #[default]
    Vertical,
    Horizonal,
    Layered,
}

impl Stack {
    fn new(contents: impl TupleViews, mode: StackMode) -> Self {
        Self {
            contents: contents.into_views(),
            mode,
        }
    }

    pub fn vertical(contents: impl TupleViews) -> Self {
        Self::new(contents, StackMode::Vertical)
    }

    pub fn horizonal(contents: impl TupleViews) -> Self {
        Self::new(contents, StackMode::Horizonal)
    }

    pub fn layered(contents: impl TupleViews) -> Self {
        Self::new(contents, StackMode::Layered)
    }
}

raw_view!(Stack);

pub fn vstack(contents: impl TupleViews) -> Stack {
    Stack::vertical(contents)
}

pub fn hstack(contents: impl TupleViews) -> Stack {
    Stack::horizonal(contents)
}

pub fn zstack(contents: impl TupleViews) -> Stack {
    Stack::layered(contents)
}

macro_rules! impl_stack {
    ($name:ident,$mode:ident) => {
        #[derive(Debug)]
        pub struct $name(Vec<AnyView>);

        impl $name {
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
