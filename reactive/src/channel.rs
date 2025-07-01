use core::cell::RefCell;
use std::rc::Rc;

use waterui_task::MainValue;

use crate::{
    Compute,
    watcher::{Metadata, Watcher, WatcherGuard, WatcherManager},
};

/// A sender for cross-thread reactive communication.
///
/// This sender can be used to send values across thread boundaries
/// while maintaining reactive behavior.
#[derive(Debug)]
pub struct Sender<T: 'static + Clone>(MainValue<Rc<Shared<T>>>);

/// A local sender for same-thread reactive communication.
///
/// This sender is optimized for use within the same thread.
#[derive(Debug, Clone)]
pub struct LocalSender<T: 'static + Clone>(Rc<Shared<T>>);

/// A receiver for reactive communication channels.
///
/// The receiver can be used to observe values sent through the channel.
#[derive(Debug, Clone)]
pub struct Receiver<T: 'static + Clone>(Rc<Shared<T>>);

#[derive(Debug, Default)]
struct Shared<T> {
    value: RefCell<T>,
    watchers: WatcherManager<T>,
}

/// Creates a channel for cross-thread reactive communication.
///
/// Returns a tuple of (sender, receiver) where the sender can be used
/// across threads and the receiver observes the values.
#[must_use]
pub fn channel<T: Send + Default + Clone>() -> (Sender<T>, Receiver<T>) {
    let shared: Rc<Shared<T>> = Rc::default();
    (Sender(MainValue::new(shared.clone())), Receiver(shared))
}

/// Creates a channel for same-thread reactive communication.
///
/// Returns a tuple of `(local_sender, receiver)` optimized for single-thread use.
#[must_use]
pub fn local_channel<T: Send + Default + Clone>() -> (LocalSender<T>, Receiver<T>) {
    let shared: Rc<Shared<T>> = Rc::default();
    (LocalSender(shared.clone()), Receiver(shared))
}

impl<T: Send + Clone + 'static> Sender<T> {
    /// Sends a value through the channel.
    ///
    /// This will notify any receivers observing this channel.
    pub fn send(&self, value: impl Into<T>) {
        let value = value.into();
        self.0.handle(move |shared| shared.value.replace(value));
    }

    /// Creates a clone of this sender for use in async contexts.
    pub async fn clone(&self) -> Self {
        Self(self.0.clone().await)
    }
}

impl<T: Send + Clone + 'static> LocalSender<T> {
    /// Sends a value through the local channel.
    ///
    /// This will notify any receivers observing this channel.
    pub fn send_with(&self, value: impl Into<T>) {
        let value = value.into();
        self.0.value.replace(value.clone());
        self.0
            .watchers
            .notify(move || value.clone(), &Metadata::new());
    }
}

impl<T: 'static + Clone> Compute for Receiver<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.0.value.borrow().clone()
    }
    fn add_watcher(&self, watcher: impl Watcher<Self::Output>) -> crate::watcher::WatcherGuard {
        WatcherGuard::from_id(&self.0.watchers, self.0.watchers.register(watcher))
    }
}
