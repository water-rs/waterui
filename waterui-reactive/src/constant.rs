use crate::{
    compute::ComputeResult,
    watcher::{Watcher, WatcherGuard},
    Compute,
};

#[derive(Debug, Clone)]
pub struct Constant<T: ComputeResult>(T);

impl<T: ComputeResult> From<T> for Constant<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T: ComputeResult> Compute for Constant<T> {
    const CONSTANT: bool = true;
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.0.clone()
    }

    fn watch(&self, _watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard {
        WatcherGuard::new(|| {})
    }
}

pub fn constant<T: ComputeResult>(value: T) -> Constant<T> {
    Constant::from(value)
}
