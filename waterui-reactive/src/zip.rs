use alloc::rc::Rc;

use crate::{
    compute::ComputeResult,
    watcher::{Watcher, WatcherGuard},
    Compute, ComputeExt,
};

#[derive(Clone)]
pub struct Zip<A, B> {
    a: A,
    b: B,
}

impl<A, B> Zip<A, B> {
    pub fn new(a: A, b: B) -> Self {
        Self { a, b }
    }
}

pub trait FlattenMap<F, T, Output: ComputeResult> {
    fn flatten_map(self, f: F) -> impl Compute<Output = Output>;
}

impl<C, F, T1, T2, Output> FlattenMap<F, (T1, T2), Output> for C
where
    C: Compute<Output = (T1, T2)>,
    F: 'static + Fn(T1, T2) -> Output,
    Output: ComputeResult,
{
    fn flatten_map(self, f: F) -> impl Compute<Output = Output> {
        self.map(move |(t1, t2)| f(t1, t2))
    }
}

impl<C, F, T1, T2, T3, Output> FlattenMap<F, (T1, T2, T3), Output> for C
where
    C: Compute<Output = ((T1, T2), T3)>,
    F: 'static + Fn(T1, T2, T3) -> Output,
    Output: ComputeResult,
{
    fn flatten_map(self, f: F) -> impl Compute<Output = Output> {
        self.map(move |((t1, t2), t3)| f(t1, t2, t3))
    }
}

pub fn zip<A, B>(a: A, b: B) -> Zip<A, B> {
    Zip::new(a, b)
}

impl<A: Compute + 'static, B: Compute + 'static> Compute for Zip<A, B> {
    type Output = (A::Output, B::Output);
    fn compute(&self) -> Self::Output {
        let Self { a, b } = self;
        (a.compute(), b.compute())
    }

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        let watcher = Rc::new(watcher);
        let Self { a, b } = self;
        let guard_a = {
            let watcher = watcher.clone();
            let b = b.clone();
            self.a
                .add_watcher(Watcher::new(move |value: A::Output, metadata| {
                    let result = (value, b.compute());
                    watcher.notify_with_metadata(result, metadata)
                }))
        };

        let guard_b = {
            let a = a.clone();
            self.b
                .add_watcher(Watcher::new(move |value: B::Output, metadata| {
                    let result = (a.compute(), value);
                    watcher.notify_with_metadata(result, metadata)
                }))
        };

        WatcherGuard::new(move || {
            let _ = (guard_a, guard_b);
        })
    }
}
