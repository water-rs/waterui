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
    fn add_watcher(&self, watcher: crate::watcher::Watcher<Self::Output>) -> WatcherGuard {
        let watcher = Rc::new(watcher);
        let a = self.a.clone();

        let b = self.b.clone();

        let guard_a = {
            let watcher = watcher.clone();
            self.a.add_watcher(Watcher::new(move |value, metadata| {
                watcher.notify_with_metadata((value, b.compute()), metadata)
            }))
        };

        let guard_b = self.b.add_watcher(Watcher::new(move |value, metadata| {
            watcher.notify_with_metadata((a.compute(), value), metadata)
        }));

        WatcherGuard::new(move || {
            let _ = (guard_a, guard_b);
        })
    }
}
