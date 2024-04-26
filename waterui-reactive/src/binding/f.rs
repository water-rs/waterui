use core::num::NonZeroUsize;

use crate::{subscriber::SubscriberManager, Compute, Reactive, Subscriber};

use super::BindingImpl;

pub struct FnBinding<Getter, Setter> {
    getter: Getter,
    setter: Setter,
    subscribers: SubscriberManager,
}

impl<Getter, Setter> FnBinding<Getter, Setter> {
    pub fn new(getter: Getter, setter: Setter) -> Self {
        Self {
            getter,
            setter,
            subscribers: SubscriberManager::new(),
        }
    }
}

impl<T, Getter, Setter> BindingImpl<T> for FnBinding<Getter, Setter>
where
    Getter: Fn() -> T,
    Setter: Fn(T),
{
    fn set(&self, value: T) {
        (self.setter)(value);
        self.notify();
    }
}

impl<T, Getter, Setter> Compute for FnBinding<Getter, Setter>
where
    Getter: Fn() -> T,
{
    type Output = T;
    fn compute(&self) -> T {
        (self.getter)()
    }
}

impl<Getter, Setter> Reactive for FnBinding<Getter, Setter> {
    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.subscribers.cancel(id)
    }

    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize> {
        Some(self.subscribers.register(subscriber))
    }
    fn notify(&self) {
        self.subscribers.notify();
    }
}

pub struct FnBindingWithState<State, Getter, Setter> {
    state: State,
    getter: Getter,
    setter: Setter,
    subscribers: SubscriberManager,
}

impl<State, Getter, Setter> FnBindingWithState<State, Getter, Setter> {
    pub fn new(state: State, getter: Getter, setter: Setter) -> Self {
        Self {
            state,
            getter,
            setter,
            subscribers: SubscriberManager::new(),
        }
    }
}

impl<State, T, Getter, Setter> BindingImpl<T> for FnBindingWithState<State, Getter, Setter>
where
    Getter: Fn(&State) -> T,
    Setter: Fn(&State, T),
{
    fn set(&self, value: T) {
        (self.setter)(&self.state, value);
        self.notify();
    }
}

impl<State, T, Getter, Setter> Compute for FnBindingWithState<State, Getter, Setter>
where
    Getter: Fn(&State) -> T,
{
    type Output = T;
    fn compute(&self) -> T {
        (self.getter)(&self.state)
    }
}

impl<State, Getter, Setter> Reactive for FnBindingWithState<State, Getter, Setter> {
    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.subscribers.cancel(id)
    }

    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize> {
        Some(self.subscribers.register(subscriber))
    }
    fn notify(&self) {
        self.subscribers.notify();
    }
}
