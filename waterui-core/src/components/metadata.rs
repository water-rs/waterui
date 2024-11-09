use crate::{AnyView, Environment, View};

#[derive(Debug)]
#[must_use]
pub struct Metadata<T: 'static> {
    pub content: AnyView,
    pub value: T,
}

impl<T> Metadata<T> {
    pub fn new(content: impl View, value: T) -> Self {
        Self {
            content: AnyView::new(content),
            value,
        }
    }
}

impl<T: 'static> View for Metadata<T> {
    fn body(self, _env: &Environment) -> impl View {
        panic!(
            "The metadata `{}`is not caught by your renderer. If the metadata is not essential, use `IgnorableMetadata<T>`.",
            core::any::type_name::<Self>()
        );
    }
}
#[derive(Debug)]
pub struct IgnorableMetadata<T> {
    pub content: AnyView,
    pub value: T,
}

impl<T: 'static> View for IgnorableMetadata<T> {
    fn body(self, _env: &Environment) -> impl View {}
}
