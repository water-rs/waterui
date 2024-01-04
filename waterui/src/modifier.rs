use crate::{component::AnyView, Environment, IntoReactive, Reactive, View};
pub struct Modifier<T: 'static> {
    pub(crate) content: AnyView,
    pub(crate) modifier: Reactive<T>,
}

impl<T: ViewModifier> Modifier<T> {
    pub fn new(content: AnyView, modifier: impl IntoReactive<T>) -> Self {
        Self {
            content,
            modifier: modifier.into_reactive(),
        }
    }
}

pub trait ViewModifier: Send + Sync + Clone + 'static {}

impl<T: Send + Sync> View for Modifier<T> {
    fn body(self, _env: Environment) -> impl View {
        panic!("You cannot call `view` for a raw view");
    }
}

#[derive(Clone)]
#[repr(transparent)]
pub struct Display(bool);
impl ViewModifier for Display {}

impl Display {
    pub fn new(condition: bool) -> Self {
        Self(condition)
    }
}
