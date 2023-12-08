use crate::view::BoxView;

pub struct TapGesture {
    pub view: BoxView,
    pub event: Box<dyn Fn() + 'static>,
}

impl TapGesture {
    pub fn new(view: BoxView, event: Box<dyn Fn() + 'static>) -> Self {
        Self { view, event }
    }
}

native_implement!(TapGesture);
