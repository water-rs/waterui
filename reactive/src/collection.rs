use core::cell::RefCell;

use alloc::vec::Vec;

use crate::{
    compute::ComputeResult,
    watcher::{Watcher, WatcherGuard, WatcherManager},
};

pub trait Collection {
    type Item;
    fn get(&self, index: usize) -> Option<Self::Item>;
    fn remove(&self, index: usize);
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn add_watcher(&self, watcher: Watcher<()>) -> WatcherGuard;
}

pub struct Array<T> {
    inner: RefCell<Vec<T>>,
    watchers: WatcherManager<()>,
}

impl<T: ComputeResult> Collection for Array<T> {
    type Item = T;
    fn get(&self, index: usize) -> Option<Self::Item> {
        self.inner.borrow().get(index).cloned()
    }
    fn remove(&self, index: usize) {
        if index < self.len() {
            self.inner.borrow_mut().remove(index);
            self.watchers.notify(());
        }
    }

    fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    fn add_watcher(&self, watcher: Watcher<()>) -> WatcherGuard {
        WatcherGuard::from_id(&self.watchers, self.watchers.register(watcher))
    }
}
