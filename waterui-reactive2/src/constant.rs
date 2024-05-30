use crate::{
    watcher::{Watcher, WatcherGuard},
    Compute,
};

#[derive(Debug, Clone)]
pub struct Constant<T: 'static + Clone>(T);

impl<T: Clone> From<T> for Constant<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T: Clone> Compute for Constant<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.0.clone()
    }

    fn add_watcher(&self, _watcher: Watcher<Self::Output>) -> WatcherGuard {
        WatcherGuard::new(|| {})
    }
}

pub fn constant<T: Clone>(value: T) -> Constant<T> {
    Constant::from(value)
}
