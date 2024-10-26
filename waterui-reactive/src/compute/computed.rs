use core::ops::Add;

use alloc::boxed::Box;

use crate::{
    constant,
    watcher::{Watcher, WatcherGuard},
};

use super::{Compute, ComputeExt, ComputeResult};

trait ComputedImpl {
    type Output: ComputeResult;
    fn compute(&self) -> Self::Output;
    fn watch(&self, watcher: Watcher<Self::Output>) -> WatcherGuard;
    fn cloned(&self) -> Computed<Self::Output>;
}

impl<C: Compute + 'static> ComputedImpl for C
where
    C: 'static,
    C::Output: 'static,
{
    type Output = C::Output;
    fn compute(&self) -> Self::Output {
        Compute::compute(self)
    }

    fn watch(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        Compute::watch(self, watcher)
    }
    fn cloned(&self) -> Computed<Self::Output> {
        Computed::new(self.clone())
    }
}

pub struct Computed<T: 'static + Clone + PartialEq>(Box<dyn ComputedImpl<Output = T>>);

impl<T: ComputeResult> PartialEq for Computed<T> {
    fn eq(&self, other: &Self) -> bool {
        Compute::compute(&self).eq(&Compute::compute(&other))
    }
}

impl<T: ComputeResult + Eq> Eq for Computed<T> {}

impl<T: ComputeResult + PartialOrd> PartialOrd for Computed<T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Compute::compute(&self).partial_cmp(&Compute::compute(&other))
    }
}

impl<T: ComputeResult + Ord> Ord for Computed<T> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        Compute::compute(&self).cmp(&Compute::compute(&other))
    }
}

impl<T: Add + ComputeResult> Add for Computed<T>
where
    T::Output: ComputeResult,
{
    type Output = Computed<T::Output>;
    fn add(self, rhs: Self) -> Self::Output {
        (self, rhs).map(|(left, right)| left + right).computed()
    }
}

impl<T: 'static + Add + ComputeResult> Add<T> for Computed<T>
where
    T::Output: ComputeResult,
{
    type Output = Computed<T::Output>;
    fn add(self, rhs: T) -> Self::Output {
        ComputeExt::map(&self, move |this| this + rhs.clone()).computed()
    }
}

impl<T: ComputeResult + Default> Default for Computed<T> {
    fn default() -> Self {
        Self::constant(T::default())
    }
}

impl<T: ComputeResult> core::fmt::Debug for Computed<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(core::any::type_name::<Self>())
    }
}

impl<T: ComputeResult> Compute for Computed<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.0.compute()
    }

    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard {
        self.0.watch(watcher.into())
    }
}

impl<T: ComputeResult> Clone for Computed<T> {
    fn clone(&self) -> Self {
        self.0.cloned()
    }
}

impl<T: ComputeResult> Computed<T> {
    pub fn new(value: impl Compute<Output = T> + 'static) -> Self {
        Self(Box::new(value))
    }
}

impl<T: ComputeResult> Computed<T> {
    pub fn constant(value: T) -> Self {
        Self::new(constant(value))
    }
}
