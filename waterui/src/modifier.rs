use crate::{component::AnyView, Compute, Computed, Environment, View};
pub struct Modifier<T: 'static> {
    pub _content: AnyView,
    pub _modifier: Computed<T>,
}

impl<T: ViewModifier> Modifier<T> {
    pub fn new(content: AnyView, modifier: impl Compute<T>) -> Self {
        Self {
            _content: content,
            _modifier: modifier.computed(),
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
