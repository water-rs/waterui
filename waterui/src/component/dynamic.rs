use core::ops::Deref;

use crate::{AnyView, View};

use waterui_core::raw_view;
use waterui_reactive::mpsc::{channel, Receiver, Sender};

pub struct DynamicHandle(Sender<AnyView>);

impl DynamicHandle {
    pub fn set(&self, content: impl View) {
        self.0.send(AnyView::new(content));
    }
}

pub struct Dynamic(Receiver<AnyView>);

impl Deref for Dynamic {
    type Target = Receiver<AnyView>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Dynamic {
    pub fn from_receiver(receiver: Receiver<AnyView>) -> Self {
        Self(receiver)
    }

    pub fn new() -> (Self, DynamicHandle) {
        let (sender, receiver) = channel();
        (Self(receiver), DynamicHandle(sender))
    }
}

raw_view!(Dynamic);
