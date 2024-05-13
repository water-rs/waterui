use crate::{
    subscriber::{Subscriber, SubscriberId},
    Compute, Reactive,
};

// DO NOT USE INTERIOR MUTABILITY!!! Reactive will fail!
pub struct Constant<T>(T);

impl<T> Constant<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }
}

impl<T: Clone> Compute for Constant<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.0.clone()
    }
}

impl<T> Reactive for Constant<T> {
    fn register_subscriber(&self, _subscriber: Subscriber) -> Option<SubscriberId> {
        None
    }
    fn cancel_subscriber(&self, _id: SubscriberId) {}
}

pub fn constant<T: Clone>(value: T) -> Constant<T> {
    Constant::new(value)
}
