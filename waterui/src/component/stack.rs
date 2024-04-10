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
pub struct Stack {
    pub _views: Vec<AnyView>,
    pub _mode: StackMode,
}

#[derive(Debug)]
#[repr(C)]
pub enum StackMode {
    Vertical,
    Horizonal,
}

impl Stack {
    pub fn new(contents: impl IntoViews) -> Self {
        Self {
            _views: contents.into_views(),
            _mode: StackMode::Vertical,
        }
    }

    pub fn vertical(mut self) -> Self {
        self._mode = StackMode::Vertical;
        self
    }

    pub fn horizonal(mut self) -> Self {
        self._mode = StackMode::Horizonal;
        self
    }
}

raw_view!(Stack);

#[derive(Debug)]
pub struct VStack {
    contents: Vec<AnyView>,
}

impl VStack {
    pub fn new(contents: impl IntoViews) -> Self {
        Self {
            contents: contents.into_views(),
        }
    }
}

#[derive(Debug)]
pub struct HStack {
    contents: Vec<AnyView>,
}

impl HStack {
    pub fn new(contents: impl IntoViews) -> Self {
        Self {
            contents: contents.into_views(),
        }
    }
}

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

pub fn stack(contents: impl IntoViews) -> Stack {
    Stack::new(contents)
}

pub fn vstack(contents: impl IntoViews) -> VStack {
    VStack::new(contents)
}

pub fn hstack(contents: impl IntoViews) -> HStack {
    HStack::new(contents)
}
