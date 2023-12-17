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

pub trait ViewModifier {}

impl<T> View for Modifier<T> {
    fn view(self) -> BoxView {
        panic!("You cannot call `view` for a raw view");
    }
}
