use core::{marker::PhantomData, num::NonZeroUsize};

use crate::{Binding, Compute, Subscriber};

use super::BindingImpl;

pub struct BirdgeBinding<Source, T, From, To> {
    source: Binding<Source>,
    from: From,
    to: To,
    _marker: PhantomData<T>,
}

impl<Source, T, From, To> BirdgeBinding<Source, T, From, To> {
    pub fn new(source: Binding<Source>, from: From, to: To) -> Self {
        Self {
            source,
            from,
            to,
            _marker: PhantomData,
        }
    }
}

impl<Source, T, From, To> Compute for BirdgeBinding<Source, T, From, To>
where
    From: Fn(T) -> Source,
    To: Fn(Source) -> T,
{
    type Output = T;
    fn compute(&self) -> Self::Output {
        (self.to)(self.source.compute())
    }
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize> {
        self.source.register_subscriber(subscriber)
    }
    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.source.cancel_subscriber(id)
    }
    fn notify(&self) {
        self.source.notify();
    }
}

impl<Source, T, From, To> BindingImpl<T> for BirdgeBinding<Source, T, From, To>
where
    From: Fn(T) -> Source,
    To: Fn(Source) -> T,
{
    fn set(&self, value: T) {
        self.source.set((self.from)(value));
    }
}

pub struct BirdgeBindingWithState<State, Source, T, From, To> {
    state: State,
    source: Binding<Source>,
    from: From,
    to: To,
    _marker: PhantomData<T>,
}

impl<State, Source, T, From, To> BirdgeBindingWithState<State, Source, T, From, To> {
    pub fn new(state: State, source: Binding<Source>, from: From, to: To) -> Self {
        Self {
            state,
            source,
            from,
            to,
            _marker: PhantomData,
        }
    }
}

impl<State, Source, T, From, To> Compute for BirdgeBindingWithState<State, Source, T, From, To>
where
    From: Fn(&State, T) -> Source,
    To: Fn(&State, Source) -> T,
{
    type Output = T;
    fn compute(&self) -> Self::Output {
        (self.to)(&self.state, self.source.compute())
    }
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize> {
        self.source.register_subscriber(subscriber)
    }
    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.source.cancel_subscriber(id)
    }
    fn notify(&self) {
        self.source.notify();
    }
}

impl<State, Source, T, From, To> BindingImpl<T>
    for BirdgeBindingWithState<State, Source, T, From, To>
where
    From: Fn(&State, T) -> Source,
    To: Fn(&State, Source) -> T,
{
    fn set(&self, value: T) {
        self.source.set((self.from)(&self.state, value));
    }
}
