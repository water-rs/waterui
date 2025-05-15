use core::cell::RefCell;
use std::rc::Rc;

use waterui_task::MainValue;

use crate::{
    Compute,
    watcher::{Metadata, Watcher, WatcherGuard, WatcherManager},
};

pub struct Sender<T: 'static + Clone>(MainValue<Rc<Shared<T>>>);

#[derive(Debug, Clone)]
pub struct LocalSender<T: 'static + Clone>(Rc<Shared<T>>);

#[derive(Debug, Clone)]
pub struct Receiver<T: 'static + Clone>(Rc<Shared<T>>);

#[derive(Debug, Default)]
struct Shared<T> {
    value: RefCell<T>,
    watchers: WatcherManager<T>,
}

pub fn channel<T: Send + Default + Clone>() -> (Sender<T>, Receiver<T>) {
    let shared: Rc<Shared<T>> = Rc::default();
    (Sender(MainValue::new(shared.clone())), Receiver(shared))
}

pub fn local_channel<T: Send + Default + Clone>() -> (LocalSender<T>, Receiver<T>) {
    let shared: Rc<Shared<T>> = Rc::default();
    (LocalSender(shared.clone()), Receiver(shared))
}

impl<T: Send + Clone + 'static> Sender<T> {
    pub fn send(&self, value: impl Into<T>) {
        let value = value.into();
        self.0.handle(move |shared| shared.value.replace(value));
    }

    pub async fn clone(&self) -> Self {
        Self(self.0.clone().await)
    }
}

impl<T: Send + Clone + 'static> LocalSender<T> {
    pub fn send_with(&self, value: impl Into<T>) {
        let value = value.into();
        self.0.value.replace(value.clone());
        self.0
            .watchers
            .notify(move || value.clone(), Metadata::new());
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
