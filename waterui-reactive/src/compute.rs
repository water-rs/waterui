use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
    ops::Deref,
    sync::Arc,
};

use crate::{
    subscriber::{SubscribeGuard, SubscribeManage, SubscriberManager},
    Binding, Subscriber,
};

pub trait Compute<T>: Send + Sync {
    fn compute(&self) -> T;
    fn register_subscriber(&self, subscriber: Subscriber) -> usize;
    fn cancel_subscriber(&self, id: usize);
}

impl<T: Debug + 'static> Debug for Computed<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Compute::<T>::compute(self).fmt(f)
    }
}

impl<T: Display + 'static> Display for Computed<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Compute::<T>::compute(self).fmt(f)
    }
}

pub trait ComputeExt<T> {
    fn subscribe(&self, subscriber: impl Into<Subscriber>) -> SubscribeGuard<'_, Self, T>
    where
        Self: Sized + Compute<T>;

    fn transform<Output>(
        self,
        transformer: impl 'static + Send + Sync + Fn(T) -> Output,
    ) -> impl Compute<Output>
    where
        T: Send + Sync + 'static;

    fn computed(self) -> Computed<T>
    where
        Self: 'static;
}

impl<V, T> ComputeExt<T> for V
where
    Self: Compute<T> + Sized,
{
    fn subscribe(&self, subscriber: impl Into<Subscriber>) -> SubscribeGuard<'_, Self, T> {
        SubscribeGuard::new(self, self.register_subscriber(subscriber.into()))
    }

    fn transform<Output>(
        self,
        transformer: impl 'static + Send + Sync + Fn(T) -> Output,
    ) -> impl Compute<Output>
    where
        T: Send + Sync + 'static,
    {
        ComputeTransformer::new(self, transformer)
    }

    fn computed(self) -> Computed<T>
    where
        Self: 'static,
    {
        Computed::from_compute(self)
    }
}

pub struct Computed<T> {
    inner: Arc<dyn Compute<T>>,
}

impl<T> Clone for Computed<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T: 'static> Compute<T> for Computed<T> {
    fn compute(&self) -> T {
        Compute::compute(self.inner.deref())
    }

    fn register_subscriber(&self, subscriber: Subscriber) -> usize {
        Compute::register_subscriber(self.inner.deref(), subscriber)
    }

    fn cancel_subscriber(&self, id: usize) {
        Compute::cancel_subscriber(self.inner.deref(), id)
    }
}

impl<T: Send + Sync + Clone + 'static> Compute<T> for T {
    fn compute(&self) -> T {
        self.clone()
    }

    fn register_subscriber(&self, _subscriber: Subscriber) -> usize {
        0
    }

    fn cancel_subscriber(&self, _id: usize) {}
}

pub struct GroupedCompute<V, const VLEN: usize, T>
where
    V: SubscribeManage<T, VLEN>,
{
    value: V,
    guards: [usize; VLEN],
    subscribers: Arc<SubscriberManager>,
    _marker: PhantomData<T>,
}

impl<V1, V2, T1, T2> GroupedCompute<(V1, V2), 2, (T1, T2)>
where
    T1: Send + Sync + 'static,
    T2: Send + Sync + 'static,
    V1: Compute<T1>,
    V2: Compute<T2>,
{
    pub fn new(v1: V1, v2: V2) -> Self {
        let subscribers = Arc::new(SubscriberManager::new());
        let value = (v1, v2);
        let guards = value.register_subscriber(|| {
            let subscribers = subscribers.clone();
            Subscriber::new(move || subscribers.notify())
        });

        Self {
            value,
            subscribers,
            guards,
            _marker: PhantomData,
        }
    }
}

impl<V, const VLEN: usize, T> Drop for GroupedCompute<V, VLEN, T>
where
    V: SubscribeManage<T, VLEN>,
{
    fn drop(&mut self) {
        self.value.cancel_subscriber(self.guards)
    }
}

impl<V1, V2, T1, T2> Compute<(T1, T2)> for GroupedCompute<(V1, V2), 2, (T1, T2)>
where
    T1: Send + Sync + 'static,
    T2: Send + Sync + 'static,
    V1: Compute<T1>,
    V2: Compute<T2>,
{
    fn compute(&self) -> (T1, T2) {
        (self.value.0.compute(), self.value.1.compute())
    }

    fn register_subscriber(&self, subscriber: Subscriber) -> usize {
        self.subscribers.register_subscriber(subscriber)
    }

    fn cancel_subscriber(&self, id: usize) {
        self.subscribers.cancel_subscriber(id)
    }
}

impl<T> Computed<T> {
    pub fn from_compute(compute: impl Compute<T> + 'static) -> Self {
        Self {
            inner: Arc::new(compute),
        }
    }
}

impl<T: Send + Clone + Sync + 'static> Compute<T> for Binding<T> {
    fn compute(&self) -> T {
        Compute::compute(&self)
    }

    fn register_subscriber(&self, subscriber: Subscriber) -> usize {
        <&Self as Compute<T>>::register_subscriber(&self, subscriber)
    }

    fn cancel_subscriber(&self, id: usize) {
        <&Self as Compute<T>>::cancel_subscriber(&self, id)
    }
}

impl<'a, T: Send + Sync + Clone> Compute<T> for &'a Binding<T> {
    fn compute(&self) -> T {
        Binding::get(self)
    }

    fn register_subscriber(&self, subscriber: Subscriber) -> usize {
        Binding::register_subscriber(self, subscriber)
    }

    fn cancel_subscriber(&self, id: usize) {
        Binding::cancel_subscriber(self, id)
    }
}

impl<T: Default + Send + Sync + 'static> Compute<T> for Binding<Option<T>> {
    fn compute(&self) -> T {
        Binding::take(self).unwrap_or_default()
    }

    fn register_subscriber(&self, subscriber: Subscriber) -> usize {
        Binding::register_subscriber(self, subscriber)
    }

    fn cancel_subscriber(&self, id: usize) {
        Binding::cancel_subscriber(self, id)
    }
}

struct ComputeTransformer<V, T, F> {
    value: V,
    transformer: F,
    _marker: PhantomData<T>,
}

impl<V, T, F> ComputeTransformer<V, T, F> {
    pub fn new<Output>(value: V, transformer: F) -> Self
    where
        F: Fn(T) -> Output,
        V: Compute<T>,
    {
        Self {
            value,
            transformer,
            _marker: PhantomData,
        }
    }
}

impl<V, T, F, Output> Compute<Output> for ComputeTransformer<V, T, F>
where
    F: 'static + Send + Sync + Fn(T) -> Output,
    V: Compute<T>,
    T: Send + Sync + 'static,
{
    fn compute(&self) -> Output {
        (self.transformer)(self.value.compute())
    }
    fn register_subscriber(&self, subscriber: Subscriber) -> usize {
        V::register_subscriber(&self.value, subscriber)
    }

    fn cancel_subscriber(&self, id: usize) {
        V::cancel_subscriber(&self.value, id)
    }
}
