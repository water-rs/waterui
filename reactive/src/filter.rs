use alloc::rc::Rc;

use crate::{
    Compute,
    watcher::{Watcher, WatcherGuard},
};

#[derive(Debug)]
pub struct Filter<C, F> {
    source: C,
    filter: Rc<F>,
}

impl<C: ComputeResult, F> Clone for Filter<C, F> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            filter: self.filter.clone(),
        }
    }
}

impl<C, F> Compute for Filter<C, F>
where
    C: Compute,
    C::Output: Default,
    F: 'static + Fn(C::Output) -> bool,
{
    type Output = C::Output;
    fn compute(&self) -> Self::Output {
        let result = self.source.compute();
        if (self.filter)(result.clone()) {
            result
        } else {
            Default::default()
        }
    }
    fn add_watcher(&self, watcher: impl Watcher<Self::Output>) -> WatcherGuard {
        self.source.watch(move |value, metadata| {
            if (self.filter)(value.clone()) {
                watcher.notify(value, metadata);
            }
        })
    }
}
