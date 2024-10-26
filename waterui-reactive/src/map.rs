use core::{cell::RefCell, ops::Deref};

use alloc::rc::Rc;

use crate::{
    compute::ComputeResult,
    watcher::{Watcher, WatcherGuard},
    Compute,
};

pub struct MapInner<C, F, Output> {
    source: C,
    f: F,
    cache: RefCell<Option<Output>>,
    _guard: RefCell<Option<WatcherGuard>>,
}

pub struct Map<C, F, Output>(Rc<MapInner<C, F, Output>>);

impl<C: Compute + 'static, F: 'static, Output: ComputeResult> Map<C, F, Output> {
    pub fn new(source: C, f: F) -> Self {
        let inner = Rc::new(MapInner {
            source,
            cache: RefCell::default(),
            f,
            _guard: RefCell::default(),
        });

        {
            let rc = inner.clone();
            let guard = inner.source.watch(Watcher::new(move |_value, _metadata| {
                rc.cache.replace(None);
            }));
            inner._guard.replace(Some(guard));
        }

        Self(inner)
    }
}

impl<C: Clone, F, Output: Clone> Clone for Map<C, F, Output> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<C, F, Output> Compute for Map<C, F, Output>
where
    C: Compute + 'static,
    Output: ComputeResult,
    F: 'static + Fn(C::Output) -> Output,
{
    type Output = Output;
    fn compute(&self) -> Self::Output {
        let this = &self.0;
        let mut cache = this.cache.borrow_mut();
        if let Some(cache) = cache.deref() {
            cache.clone()
        } else {
            let result = (this.f)(this.source.compute());
            cache.replace(result.clone());
            result
        }
    }

    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard {
        let watcher: Watcher<_> = watcher.into();
        let this = self.clone();
        self.0.source.watch(Watcher::new(move |_value, metadata| {
            watcher.notify_with_metadata(this.compute(), metadata)
        }))
    }
}
