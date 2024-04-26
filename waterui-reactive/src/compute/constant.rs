use core::num::NonZeroUsize;

use crate::{subscriber::BoxSubscriber, Compute, Reactive};
pub struct Constant<T> {
    value: T,
}

impl<T> Constant<T> {
    pub fn new(value: T) -> Self {
        Self { value }
    }
}

impl<T: Clone> Compute for Constant<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.value.clone()
    }
}

impl<T> Reactive for Constant<T> {
    fn register_subscriber(&self, _subscriber: BoxSubscriber) -> Option<NonZeroUsize> {
        None
    }
    fn cancel_subscriber(&self, _id: NonZeroUsize) {}
    fn notify(&self) {}
}
