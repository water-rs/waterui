use std::{ops::Deref, sync::Arc};

use async_broadcast::{broadcast, Receiver, Sender};

#[derive(Debug, Clone)]
pub struct Binding<T: ?Sized> {
    value: Arc<T>,
    sender: Sender<Arc<T>>,
    receiver: Receiver<Arc<T>>,
}

impl<T> Deref for Binding<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.value.as_ref()
    }
}

impl<T: Clone> Binding<T> {
    pub fn into_wrapped(self) -> T {
        match Arc::try_unwrap(self.value) {
            Ok(value) => value,
            Err(s) => s.deref().clone(),
        }
    }
}

pub trait IntoBinding<T: ?Sized> {
    fn into_binding(self) -> Binding<T>;
}

impl<T, V: Into<T>> IntoBinding<T> for V {
    fn into_binding(self) -> Binding<T> {
        Binding::new(self.into())
    }
}

pub struct Subcriber<T> {
    receiver: Receiver<Arc<T>>,
}

impl<T> Subcriber<T> {
    fn new(receiver: Receiver<Arc<T>>) -> Self {
        Self { receiver }
    }
}

impl<T> Binding<T> {
    pub fn new(value: T) -> Self {
        let (sender, receiver) = broadcast(1);
        Self {
            value: Arc::new(value),
            sender,
            receiver,
        }
    }

    pub fn subcribe(&self) -> Subcriber<T> {
        Subcriber::new(self.receiver.clone())
    }

    pub fn set(&mut self, value: T) {
        self.sender.broadcast(Arc::new(value));
    }

    pub fn on_change<F, Fut>(f: F) {}
}
