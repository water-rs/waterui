use crate::view::ViewExt;
use crate::{AnyView, Environment, View};

#[non_exhaustive]
#[derive(Debug)]
pub struct Metadata<T> {
    pub _content: AnyView,
    pub _value: T,
}

impl<T> Metadata<T> {
    pub fn new(content: impl View, value: T) -> Self {
        Self {
            _content: content.anyview(),
            _value: value,
        }
    }
}

impl<T: 'static> View for Metadata<T> {
    fn body(self, _env: Environment) -> impl View {
        panic!(
            "The metadata `{}`is not caught by your renderer. If the metadata is not essential, use `IgnorableMetadata<T>`.",
            core::any::type_name::<Self>()
        );
    }
}

pub struct IgnorableMetadata<T> {
    pub _content: AnyView,
    pub _value: T,
}

impl<T: 'static> View for IgnorableMetadata<T> {
    fn body(self, _env: Environment) -> impl View {}
}
