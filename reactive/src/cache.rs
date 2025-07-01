use core::cell::RefCell;

use alloc::rc::Rc;

use crate::{Compute, watcher::WatcherGuard};

/// A cached computation that stores the last computed value.
///
/// This wrapper around a computation caches the result and only recomputes
/// when the underlying source changes, improving performance for expensive operations.
#[derive(Debug, Clone)]
pub struct Cached<C>
where
    C: Compute,
    C::Output: Clone,
{
    source: C,
    cache: Rc<RefCell<Option<C::Output>>>,
    _guard: Rc<WatcherGuard>,
}

impl<C> Cached<C>
where
    C: Compute,
    C::Output: Clone,
{
    /// Creates a new cached computation from the given source.
    ///
    /// The cache starts empty and will be populated on the first access.
    pub fn new(source: C) -> Self {
        let cache: Rc<RefCell<Option<C::Output>>> = Rc::default();
        let guard = {
            let cache = cache.clone();
            source.add_watcher(move |value, _| {
                *cache.borrow_mut() = Some(value);
            })
        };

        Self {
            source,
            cache,
            _guard: Rc::new(guard),
        }
    }
}

impl<C> Compute for Cached<C>
where
    C: Compute,
    C::Output: Clone,
{
    type Output = C::Output;
    fn compute(&self) -> Self::Output {
        let mut cache = self.cache.borrow_mut();
        #[allow(clippy::option_if_let_else)]
        if let Some(cache) = &*cache {
            cache.clone()
        } else {
            let value = self.source.compute();
            *cache = Some(value.clone());
            value
        }
    }

    fn add_watcher(&self, watcher: impl crate::watcher::Watcher<Self::Output>) -> WatcherGuard {
        self.source.add_watcher(watcher)
    }
}

/// Creates a cached computation from the given source.
///
/// This is a convenience function equivalent to `Cached::new(source)`.
pub fn cached<C>(source: C) -> Cached<C>
where
    C: Compute,
    C::Output: Clone,
{
    Cached::new(source)
}
