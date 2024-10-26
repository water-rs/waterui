use core::ops::Deref;

use alloc::{boxed::Box, rc::Rc};

use crate::watcher::{Watcher, WatcherGuard};

use super::Compute;

impl<C: Compute> Compute for &C {
    type Output = C::Output;

    fn compute(&self) -> Self::Output {
        Compute::compute(*self)
    }

    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard {
        Compute::watch(*self, watcher)
    }
}

impl<C: Compute + 'static> Compute for Rc<C> {
    type Output = C::Output;

    fn compute(&self) -> Self::Output {
        Compute::compute(self.deref())
    }

    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard {
        Compute::watch(self.deref(), watcher)
    }
}

impl<C: Compute + 'static> Compute for Box<C> {
    type Output = C::Output;

    fn compute(&self) -> Self::Output {
        Compute::compute(self.deref())
    }

    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard {
        Compute::watch(self.deref(), watcher)
    }
}
