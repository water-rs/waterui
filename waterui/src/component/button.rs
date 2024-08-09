use core::fmt::Debug;
use core::future::Future;

use alloc::boxed::Box;

use crate::View;
use crate::{task, AnyView, ViewExt};

#[non_exhaustive]
pub struct ButtonConfig {
    pub label: AnyView,
    pub action: Action,
}

impl_debug!(ButtonConfig);

configurable!(Button, ButtonConfig);

pub type Action = Box<dyn Fn()>;

impl Default for Button {
    fn default() -> Self {
        Self(ButtonConfig {
            label: ().anyview(),
            action: Box::new(|| {}),
        })
    }
}

impl Button {
    pub fn new(label: impl View) -> Self {
        let mut button = Self::default();
        button.0.label = label.anyview();
        button
    }

    pub fn action(mut self, action: impl Fn() + 'static) -> Self {
        self.0.action = Box::new(action);
        self
    }

    pub fn action_async<Fut>(self, action: impl 'static + Fn() -> Fut) -> Self
    where
        Fut: Future + 'static,
    {
        self.action(move || {
            task(action()).detach();
        })
    }
}

pub fn button(label: impl View) -> Button {
    Button::new(label)
}
