use crate::{view::BoxView, widget, BoxEvent};

#[widget]
pub struct TapGesture {
    pub view: BoxView,
    pub event: BoxEvent,
}

impl TapGesture {
    pub fn new(view: BoxView, event: BoxEvent) -> Self {
        Self { view, event }
    }
}

native_implement!(TapGesture);
