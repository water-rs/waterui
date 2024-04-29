use alloc::{boxed::Box, vec::Vec};
use waterui_core::view::ConstantViews;
use waterui_core::view::{BoxViews, Views};

use crate::{AnyView, Environment, View};

macro_rules! impl_from_iter {
    ($($ty:ident),*) => {
        $(
            impl<V: View + 'static> FromIterator<V> for $ty {
                fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
                    let content: Vec<_> = iter.into_iter().map(|v| {AnyView::new(v)}).collect();
                    Self::new(ConstantViews::new(content))
                }
            }
        )*

    };
}

impl_from_iter!(Stack, VStack, HStack);

#[non_exhaustive]
pub struct Stack {
    pub _views: BoxViews<AnyView>,
    pub _mode: StackMode,
}

#[derive(Debug)]
pub enum StackMode {
    Auto,
    Vertical,
    Horizonal,
    Layered,
}

impl Stack {
    pub fn new(contents: impl Views<Item = AnyView> + 'static) -> Self {
        Self {
            _views: Box::new(contents),
            _mode: StackMode::Auto,
        }
    }

    pub fn vertical(self) -> Self {
        self.mode(StackMode::Vertical)
    }

    pub fn horizonal(self) -> Self {
        self.mode(StackMode::Horizonal)
    }

    pub fn layered(self) -> Self {
        self.mode(StackMode::Layered)
    }

    pub fn mode(mut self, mode: StackMode) -> Self {
        self._mode = mode;
        self
    }
}

raw_view!(Stack);

macro_rules! impl_stacks {
    ($ty:ident) => {
        pub struct $ty {
            contents: BoxViews<AnyView>,
        }

        impl $ty {
            pub fn new(contents: impl Views<Item = AnyView> + 'static) -> Self {
                Self {
                    contents: Box::new(contents),
                }
            }
        }
    };
}

impl_stacks!(VStack);

impl_stacks!(HStack);
impl_stacks!(ZStack);

impl View for VStack {
    fn body(self, _env: Environment) -> impl View {
        Stack::new(self.contents).vertical()
    }
}

impl View for HStack {
    fn body(self, _env: Environment) -> impl View {
        Stack::new(self.contents).horizonal()
    }
}

impl View for ZStack {
    fn body(self, _env: Environment) -> impl View {
        Stack::new(self.contents).layered()
    }
}

pub fn stack(contents: impl Views<Item = AnyView> + 'static) -> Stack {
    Stack::new(contents)
}

pub fn vstack(contents: impl Views<Item = AnyView> + 'static) -> VStack {
    VStack::new(contents)
}

pub fn hstack(contents: impl Views<Item = AnyView> + 'static) -> HStack {
    HStack::new(contents)
}

pub fn zstack(contents: impl Views<Item = AnyView> + 'static) -> ZStack {
    ZStack::new(contents)
}
