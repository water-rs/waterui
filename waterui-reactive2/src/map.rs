use core::{future::Future, marker::PhantomData};

use alloc::rc::Rc;
use waterui_core::Environment;

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

pub struct AsyncMap<C, F, Output> {
    source: C,
    env: Environment,
    f: Rc<F>,
    _marker: PhantomData<Output>,
}
impl<C, F, Output> AsyncMap<C, F, Output> {
    pub fn new(source: C, env: Environment, f: F) -> Self {
        Self {
            source,
            env,
            f: Rc::new(f),
            _marker: PhantomData,
        }
    }
}

impl<C: Compute, F, Output> Clone for AsyncMap<C, F, Output> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            env: self.env.clone(),
            f: self.f.clone(),
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

impl<C, F, Fut, Output> Compute for AsyncMap<C, F, Output>
where
    C: Compute,
    C::Output: 'static,
    F: 'static + Fn(C::Output) -> Fut,
    Fut: Future<Output = Output> + 'static,
    Output: 'static,
{
    type Output = Option<Output>;
    fn compute(&self) -> Self::Output {
        None
    }

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        let f = self.f.clone();
        let env = self.env.clone();
        let watcher = Rc::new(watcher);

        self.source
            .add_watcher(Watcher::new(move |value, metadata| {
                let f = f.clone();
                let watcher = watcher.clone();
                let fut = (f.clone())(value);
                let metadata = metadata.clone();
                env.task(async move {
                    let value = fut.await;
                    watcher.notify_with_metadata(Some(value), &metadata)
                })
            }))
    }
}
