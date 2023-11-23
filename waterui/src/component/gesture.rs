use crate::{view::BoxView, view::Frame, widget, BoxEvent};

#[widget]
pub struct TapGesture {
    pub view: BoxView,
    pub event: BoxEvent,
}

impl TapGesture {
    pub fn new(view: BoxView, event: BoxEvent) -> Self {
        Self {
            view,
            event,
            frame: Frame::default(),
        }
    }
}

native_implement!(TapGesture);
