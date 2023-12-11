use crate::view::BoxView;

pub struct TapGesture {
    pub view: BoxView,
    pub event: Box<dyn Fn()>,
}

impl TapGesture {
    pub fn new(view: BoxView, event: Box<dyn Fn()>) -> Self {
        Self { view, event }
    }
}

raw_view!(TapGesture);
