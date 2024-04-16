use alloc::vec::Vec;

use crate::component::AnyView;
use crate::view::IntoViews;
use crate::View;

macro_rules! impl_from_iter {
    ($($ty:ident),*) => {
        $(
            impl<V: View + 'static> FromIterator<V> for $ty {
                fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
                    let content: Vec<_> = iter.into_iter().map(|v| {AnyView::new(v)}).collect();
                    Self::new(content)
                }
            }
        )*

    };
}

impl_from_iter!(Stack, VStack, HStack);

#[derive(Debug)]
#[non_exhaustive]
pub struct Stack {
    pub _views: Vec<AnyView>,
    pub _mode: StackMode,
}

#[derive(Debug)]
#[repr(C)]
pub enum StackMode {
    Vertical,
    Horizonal,
    Layered,
}

impl Stack {
    pub fn new(contents: impl IntoViews) -> Self {
        Self {
            _views: contents.into_views(),
            _mode: StackMode::Vertical,
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
        #[derive(Debug)]
        pub struct $ty {
            contents: Vec<AnyView>,
        }

        impl $ty {
            pub fn new(contents: impl IntoViews) -> Self {
                Self {
                    contents: contents.into_views(),
                }
            }
        }
    };
}

impl_stacks!(VStack);

impl_stacks!(HStack);
impl_stacks!(ZStack);

impl View for VStack {
    fn body(self, _env: crate::Environment) -> impl View {
        Stack::new(self.contents).vertical()
    }
}

impl View for HStack {
    fn body(self, _env: crate::Environment) -> impl View {
        Stack::new(self.contents).horizonal()
    }
}

impl View for ZStack {
    fn body(self, _env: crate::Environment) -> impl View {
        Stack::new(self.contents).layered()
    }
}

pub fn stack(contents: impl IntoViews) -> Stack {
    Stack::new(contents)
}

pub fn vstack(contents: impl IntoViews) -> VStack {
    VStack::new(contents)
}

pub fn hstack(contents: impl IntoViews) -> HStack {
    HStack::new(contents)
}

pub fn zstack(contents: impl IntoViews) -> ZStack {
    ZStack::new(contents)
}
