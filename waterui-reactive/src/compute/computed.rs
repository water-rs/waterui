use core::ops::Add;

use alloc::boxed::Box;

use crate::{
    compute::ext::ComputeExt,
    constant,
    watcher::{Watcher, WatcherGuard},
    zip::FlattenMap,
};

use super::{Compute, ComputeResult};

pub struct Computed<T: ComputeResult>(Box<dyn ComputedImpl<Output = T>>);

trait ComputedImpl {
    type Output: ComputeResult;
    fn compute(&self) -> Self::Output;
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard;
    fn cloned(&self) -> Computed<Self::Output>;
}

impl<C: Compute> ComputedImpl for C {
    type Output = C::Output;
    fn compute(&self) -> Self::Output {
        <Self as Compute>::compute(self)
    }
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        <Self as Compute>::add_watcher(self, watcher)
    }
    fn cloned(&self) -> Computed<Self::Output> {
        self.clone().computed()
    }
}

impl<T: ComputeResult> Clone for Computed<T> {
    fn clone(&self) -> Self {
        self.0.cloned()
    }
}

impl<T: Add + ComputeResult> Add for Computed<T>
where
    T::Output: ComputeResult,
{
    type Output = Computed<T::Output>;
    fn add(self, rhs: Self) -> Self::Output {
        self.zip(rhs)
            .flatten_map(|left, right| left + right)
            .computed()
    }
}

impl<T: 'static + Add + ComputeResult> Add<T> for Computed<T>
where
    T::Output: ComputeResult,
{
    type Output = Computed<T::Output>;
    fn add(self, rhs: T) -> Self::Output {
        ComputeExt::map(self, move |this| this + rhs.clone()).computed()
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

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        self.0.add_watcher(watcher)
    }
}

impl<T: ComputeResult> Computed<T> {
    pub fn new<C>(value: C) -> Self
    where
        C: Compute<Output = T> + Clone + 'static,
    {
        Self(Box::new(value))
    }
}

impl<T: ComputeResult> Computed<T> {
    pub fn constant(value: T) -> Self {
        Self::new(constant(value))
    }
}
