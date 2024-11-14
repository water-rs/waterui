use core::fmt::Debug;

use alloc::boxed::Box;
use waterui_core::handler::{into_handler_mut, BoxHandlerMut, HandlerFnMut};

use crate::View;
use crate::{AnyView, ViewExt};

#[non_exhaustive]
pub struct ButtonConfig {
    pub label: AnyView,
    pub action: BoxHandlerMut<()>,
}

impl_debug!(ButtonConfig);

configurable!(Button, ButtonConfig);

impl Default for Button {
    fn default() -> Self {
        Self(ButtonConfig {
            label: ().anyview(),
            action: Box::new(into_handler_mut(|| {})),
        })
    }
}

impl Button {
    pub fn new(label: impl View) -> Self {
        let mut button = Self::default();
        button.0.label = label.anyview();
        button
    }

    pub fn action<P: 'static>(mut self, action: impl HandlerFnMut<P, ()>) -> Self {
        self.0.action = Box::new(into_handler_mut(action));
        self
    }
}

pub fn button(label: impl View) -> Button {
    Button::new(label)
}
