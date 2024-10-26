use alloc::rc::Rc;

use crate::{
    watcher::{Watcher, WatcherGuard},
    Compute,
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

pub fn zip<A, B>(a: A, b: B) -> Zip<A, B> {
    Zip::new(a, b)
}

impl<A: Compute + 'static, B: Compute + 'static> Compute for Zip<A, B> {
    type Output = (A::Output, B::Output);
    fn compute(&self) -> Self::Output {
        (self.a.compute(), self.b.compute())
    }

    fn watch(&self, watcher: impl Into<crate::watcher::Watcher<Self::Output>>) -> WatcherGuard {
        let watcher = Rc::new(watcher.into());
        let Self { a, b } = self.clone();

        let guard_a = {
            let watcher = watcher.clone();
            self.a
                .watch(Watcher::new(move |value: A::Output, metadata| {
                    let result = (value.clone(), b.compute());
                    watcher.notify_with_metadata(result, metadata)
                }))
        };

        let guard_b = self
            .b
            .watch(Watcher::new(move |value: B::Output, metadata| {
                let result = (a.compute(), value.clone());
                watcher.notify_with_metadata(result, metadata)
            }));

        WatcherGuard::new(move || {
            let _ = (guard_a, guard_b);
        })
    }
}
