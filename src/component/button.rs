use crate::reactive::Ref;

use crate::{BoxEvent, Event};

pub struct Button {
    pub label: Ref<String>,
    on_click: BoxEvent,
    on_press: BoxEvent,
}

impl Button {
    pub fn on_click(mut self, event: impl Event) -> Self {
        self.on_click = Box::new(event);
        self
    }

    pub fn on_press(mut self, event: impl Event) -> Self {
        self.on_press = Box::new(event);
        self
    }
}

native_implement!(Button);
