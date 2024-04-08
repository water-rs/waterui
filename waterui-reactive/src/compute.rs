use std::{fmt::Display, marker::PhantomData, ops::Deref, sync::Arc};

use crate::{subscriber::SharedSubscriberManager, Binding, Subscriber};

pub trait Compute<T> {
    fn get(&self) -> T;
    fn subscribe(&self, _subscriber: Subscriber) -> usize {
        0
    }
    fn unsubscribe(&self, _id: usize) {}

    fn computed(self) -> Computed<T>
    where
        Self: Sized + 'static,
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

impl<T> Compute<T> for Computed<T> {
    fn get(&self) -> T {
        Compute::get(self.inner.deref())
    }
}

impl<T: Clone> Compute<T> for T {
    fn get(&self) -> T {
        self.clone()
    }
}

impl<T> Computed<T> {
    pub fn to<F, Output>(&self, transformer: F) -> impl Compute<Output>
    where
        F: Fn(T) -> Output,
    {
        ComputedTransformer {
            value: self.clone(),
            transformer,
            _marker: PhantomData,
        }
    }

    pub fn transform<Output: From<T>>(&self) -> impl Compute<Output> {
        self.to(Into::into)
    }

    pub fn try_transform<Output: TryFrom<T>>(&self) -> impl Compute<Result<Output, Output::Error>> {
        self.to(TryInto::try_into)
    }

    pub fn from_compute(compute: impl Compute<T> + 'static) -> Self {
        Self {
            inner: Arc::new(compute),
        }
    }
}

impl<T: Display> Computed<T> {
    pub fn display(&self) -> impl Compute<String> {
        self.to(|v| v.to_string())
    }
}

struct ComputedTransformer<V, T, F> {
    value: V,
    transformer: F,
    _marker: PhantomData<T>,
}

impl<V, T, F, Output> Compute<Output> for ComputedTransformer<V, T, F>
where
    V: Compute<T>,
    F: Fn(T) -> Output,
{
    fn get(&self) -> Output {
        let t = Compute::get(&self.value);
        (self.transformer)(t)
    }
}

impl<T> Compute<T> for Binding<T> {
    fn get(&self) -> T {
        Binding::get(self)
    }

    fn subscribe(&self, subscriber: Subscriber) -> usize {
        Binding::subscribe(self, subscriber)
    }

    fn unsubscribe(&self, id: usize) {
        Binding::unsubscribe(self, id)
    }
}

impl<T1, T2> Computed<(T1, T2)> {
    pub fn compute<V1, V2, F, Output>(v1: V1, v2: V2, transformer: F) -> impl Compute<Output>
    where
        V1: Compute<T1>,
        V2: Compute<T2>,
        F: Fn(T1, T2) -> Output,
    {
        ComputedBuilder::new(v1, v2, transformer)
    }
}

struct ComputedBuilder<V, F, T> {
    values: V,
    transformer: F,
    subscribers: SharedSubscriberManager,
    _marker: PhantomData<T>,
}

impl<V1, V2, T1, T2, F, Output> ComputedBuilder<(V1, V2), F, (T1, T2)>
where
    V1: Compute<T1>,
    V2: Compute<T2>,
    F: Fn(T1, T2) -> Output,
{
    pub fn new(v1: V1, v2: V2, transformer: F) -> Self {
        let subscribers = SharedSubscriberManager::new();

        v1.subscribe({
            let subscribers = subscribers.clone();
            Subscriber::new(move || subscribers.notify())
        });

        v2.subscribe({
            let subscribers = subscribers.clone();
            Subscriber::new(move || subscribers.notify())
        });
        Self {
            values: (v1, v2),
            transformer,
            subscribers,
            _marker: PhantomData,
        }
    }
}

impl<V1, V2, T1, T2, F, Output> Compute<Output> for ComputedBuilder<(V1, V2), F, (T1, T2)>
where
    V1: Compute<T1>,
    V2: Compute<T2>,
    F: Fn(T1, T2) -> Output,
{
    fn get(&self) -> Output {
        (self.transformer)(self.values.0.get(), self.values.1.get())
    }

    fn subscribe(&self, subscriber: Subscriber) -> usize {
        self.subscribers.subscribe(subscriber)
    }

    fn unsubscribe(&self, id: usize) {
        self.subscribers.unsubscribe(id);
    }
}
