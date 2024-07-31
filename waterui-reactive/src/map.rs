use core::marker::PhantomData;

use alloc::rc::Rc;

use crate::{
    watcher::{Watcher, WatcherGuard},
    Compute,
};

pub struct Map<C, F, Output> {
    source: C,
    f: Rc<F>,
    _marker: PhantomData<Output>,
}

impl<C, F, Output> Map<C, F, Output> {
    pub fn new(source: C, f: F) -> Self {
        Self {
            source,
            f: Rc::new(f),
            _marker: PhantomData,
        }
    }
}

impl<C: Compute, F, Output> Clone for Map<C, F, Output> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            f: self.f.clone(),
            _marker: PhantomData,
        }
    }
}

impl<C, F, Output> Compute for Map<C, F, Output>
where
    C: Compute,
    Output: 'static,
    F: 'static + Fn(C::Output) -> Output,
{
    type Output = Output;
    fn compute(&self) -> Self::Output {
        (self.f)(self.source.compute())
    }

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        let f = self.f.clone();
        self.source
            .add_watcher(Watcher::new(move |value, metadata| {
                watcher.notify_with_metadata(f(value), metadata)
            }))
    }
}
