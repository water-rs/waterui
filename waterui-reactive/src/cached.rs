use core::{cell::RefCell, ops::Deref};

use crate::{
    watcher::{Watcher, WatcherGuard},
    Compute,
};

#[derive(Debug, Clone)]
pub struct Cached<C, T> {
    source: C,
    cache: RefCell<Option<T>>,
}

impl<C, T: 'static> Compute for Cached<C, T>
where
    C: Compute<Output = T>,
    T: Clone,
{
    type Output = T;
    fn compute(&self) -> Self::Output {
        if let Some(value) = self.cache.borrow().deref() {
            value.clone()
        } else {
            let value = self.source.compute();
            *self.cache.borrow_mut() = Some(value.clone());
            value
        }
    }
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        self.source.add_watcher(watcher)
    }
}
