use crate::{component::AnyView, Environment, View};
pub struct Modifier<T> {
    pub(crate) content: AnyView,
    pub(crate) modifier: T,
}

impl<T: ViewModifier> Modifier<T> {
    pub fn new(content: AnyView, modifier: T) -> Self {
        Self { content, modifier }
    }
}

pub trait ViewModifier: Send + Sync {}

impl<T: Send + Sync> View for Modifier<T> {
    fn body(self, _env: Environment) -> impl View {
        panic!("You cannot call `view` for a raw view");
    }
}
