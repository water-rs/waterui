use take_mut::take;

use crate::subscriber::Subscriber;
use std::{
    fmt::{Debug, Display},
    hash::Hash,
    ops::{Deref, DerefMut},
    sync::Arc,
    sync::{RwLock, RwLockReadGuard},
};

pub struct Reactive<T: 'static> {
    pub(crate) inner: Arc<ReactiveInner<T>>,
}

impl<T: Default> Default for Reactive<T> {
    fn default() -> Self {
        Self::new(|| T::default())
    }
}

impl<T: PartialEq> PartialEq for Reactive<T> {
    fn eq(&self, other: &Self) -> bool {
        let left = self.inner.value.read().unwrap();
        let right = other.inner.value.read().unwrap();
        left.deref() == right.deref()
    }
}

impl<T: Eq> Eq for Reactive<T> {}

impl<T: Hash> Hash for Reactive<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.get().hash(state)
    }
}

impl<T: Debug> Debug for Reactive<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}

impl<T: Display> Display for Reactive<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}

impl<T> Clone for Reactive<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Reactive<T> {
    pub fn new(updater: impl 'static + Send + Sync + Fn() -> T) -> Self {
        Self {
            inner: Arc::new(ReactiveInner::new(updater)),
        }
    }

    pub fn get(&self) -> ReactiveGuard<T> {
        self.inner.get()
    }

    pub fn on_update(&self, subscriber: impl Into<Subscriber>) {
        self.inner.on_update(subscriber)
    }

    pub fn to_with_ref<Output>(
        &self,
        f: impl 'static + Send + Sync + Fn(&T) -> Output,
    ) -> Reactive<Output>
    where
        T: Send + Sync,
    {
        let reactive = self.clone();
        let output = Reactive::new(move || f(reactive.get().deref()));
        output
    }

    pub fn to<Output>(&self, f: impl 'static + Send + Sync + Fn(T) -> Output) -> Reactive<Output>
    where
        T: Send + Sync,
    {
        let reactive = self.clone();
        Reactive::new(move || f(reactive.take()))
    }

    pub fn take(&self) -> T {
        self.inner.take()
    }

    pub fn need_update(&self) {
        self.inner.need_update()
    }

    /// Constructs a `Reactive<T>` from a raw pointer
    /// # Safety
    /// The raw pointer must have been previously returned by a call to Reactive<T>::into_raw
    pub unsafe fn from_raw(ptr: *const T) -> Self {
        unsafe {
            Self {
                inner: Arc::from_raw(ptr as *const ReactiveInner<T>),
            }
        }
    }

    /// Consumes the Reactive, returning the wrapped pointer.
    /// To avoid a memory leak the pointer must be converted back to a Reactive using Reactive::from_raw.
    pub fn into_raw(self) -> *const T {
        Arc::into_raw(self.inner) as *const T
    }
}

pub struct ReactiveGuard<'a, T> {
    guard: RwLockReadGuard<'a, Option<T>>,
}

impl<'a, T> Deref for ReactiveGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().unwrap()
    }
}

type Updater<T> = Box<dyn Send + Sync + Fn() -> T>;
pub(crate) struct ReactiveInner<T: 'static> {
    value: RwLock<Option<T>>,
    updater: RwLock<Updater<T>>,
}

impl<T> ReactiveInner<T> {
    pub fn new(updater: impl 'static + Send + Sync + Fn() -> T) -> Self {
        Self {
            value: RwLock::new(None),
            updater: RwLock::new(Box::new(updater)),
        }
    }

    pub fn get(&self) -> ReactiveGuard<T> {
        let mut guard = self.value.write().unwrap();
        if guard.is_none() {
            *guard = Some((self.updater.read().unwrap())());
        }

        ReactiveGuard {
            guard: self.value.read().unwrap(),
        }
    }

    pub fn take(&self) -> T {
        self.value
            .write()
            .unwrap()
            .take()
            .unwrap_or((self.updater.read().unwrap())())
    }

    pub fn on_update(&self, subscriber: impl Into<Subscriber>) {
        let subscriber = subscriber.into();
        take(self.updater.write().unwrap().deref_mut(), |value| {
            Box::new(move || {
                let new_value = value();
                subscriber.call();
                new_value
            })
        })
    }

    pub fn need_update(&self) {
        self.value.write().unwrap().take();
    }
}

impl<T: Clone + Send + Sync> From<T> for Reactive<T> {
    fn from(value: T) -> Self {
        Reactive::new(move || value.clone())
    }
}

pub trait IntoReactive<T> {
    fn into_reactive(self) -> Reactive<T>;
}

impl<T: Clone + Send + Sync> IntoReactive<T> for T {
    fn into_reactive(self) -> Reactive<T> {
        Reactive::new(move || self.clone())
    }
}
