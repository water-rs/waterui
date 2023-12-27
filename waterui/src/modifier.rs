use crate::view::BoxView;
use crate::View;
pub struct Modifier<T> {
    pub(crate) content: BoxView,
    pub(crate) modifier: T,
}

impl<T: ViewModifier> Modifier<T> {
    pub fn new(content: BoxView, modifier: T) -> Self {
        Self { content, modifier }
    }
}

pub trait ViewModifier: Send + Sync {}

impl<T: Send + Sync> View for Modifier<T> {
    fn body(self) -> BoxView {
        panic!("You cannot call `view` for a raw view");
    }
}
