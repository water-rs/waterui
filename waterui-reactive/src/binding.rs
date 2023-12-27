use std::{
    fmt::Debug,
    mem::replace,
    ops::{Deref, DerefMut},
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::{reactive::Reactive, subscriber::Subscriber};

pub struct Binding<T> {
    inner: Arc<BindingInner<T>>,
}

impl<T: Debug> Debug for Binding<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}
impl<T> Clone for Binding<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

struct BindingInner<T> {
    value: RwLock<T>,
    subscribers: RwLock<Vec<Subscriber>>,
}

pub struct BindingReadGuard<'a, T> {
    guard: RwLockReadGuard<'a, T>,
}

impl<T> Deref for BindingReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.deref()
    }
}

pub struct BindingWriteGuard<'a, T> {
    guard: Option<RwLockWriteGuard<'a, T>>,
    subscribers: &'a RwLock<Vec<Subscriber>>,
}

impl<T> Deref for BindingWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.as_deref().unwrap()
    }
}

impl<T> DerefMut for BindingWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_deref_mut().unwrap()
    }
}

impl<T> Drop for BindingWriteGuard<'_, T> {
    fn drop(&mut self) {
        let _ = self.guard.take();
        let _ = self
            .subscribers
            .read()
            .unwrap()
            .iter()
            .map(Subscriber::call);
    }
}

impl<T> BindingInner<T> {
    pub fn get(&self) -> BindingReadGuard<T> {
        BindingReadGuard {
            guard: self.value.read().unwrap(),
        }
    }

    pub fn get_mut(&self) -> BindingWriteGuard<T> {
        BindingWriteGuard {
            guard: Some(self.value.write().unwrap()),
            subscribers: &self.subscribers,
        }
    }

    pub fn subscribe(&self, subscriber: impl Into<Subscriber>) {
        self.subscribers.write().unwrap().push(subscriber.into())
    }
}

impl<T> Binding<T> {
    pub fn new(value: impl Into<T>) -> Self {
        Self::from(value.into())
    }

    pub fn get(&self) -> BindingReadGuard<T> {
        self.inner.get()
    }

    pub fn get_mut(&self) -> BindingWriteGuard<T> {
        self.inner.get_mut()
    }

    pub fn subscribe(&self, subscriber: impl Into<Subscriber>) {
        self.inner.subscribe(subscriber)
    }

    pub fn to<Output: 'static>(
        &self,
        f: impl 'static + Send + Sync + Fn(&T) -> Output,
    ) -> Reactive<Output>
    where
        T: Send + Sync + 'static,
        Output: Send + Sync,
    {
        let reactive = self.inner.clone();

        let output = Reactive::new(move || f(reactive.get().deref()));
        let output_weak = Arc::downgrade(&output.inner);

        self.subscribe(move || {
            if let Some(output) = output_weak.upgrade() {
                output.need_update()
            }
        });
        output
    }

    pub fn make_effect(&self) {
        let _ = self
            .inner
            .subscribers
            .read()
            .unwrap()
            .iter()
            .map(Subscriber::call);
    }

    pub fn replace(&self, value: impl Into<T>) -> T {
        replace(self.get_mut().deref_mut(), value.into())
    }

    pub fn set(&self, value: impl Into<T>) {
        let _ = self.replace(value);
    }

    /// Constructs a `Binding<T>` from a raw pointer
    /// # Safety
    /// The raw pointer must have been previously returned by a call to Binding<T>::into_raw
    pub unsafe fn from_raw(ptr: *const T) -> Self {
        unsafe {
            Self {
                inner: Arc::from_raw(ptr as *const BindingInner<T>),
            }
        }
    }

    /// Consumes the Reactive, returning the wrapped pointer.
    /// To avoid a memory leak the pointer must be converted back to a Binding using Binding::from_raw.
    pub fn into_raw(self) -> *const T {
        Arc::into_raw(self.inner) as *const T
    }
}

impl<T> From<T> for Binding<T> {
    fn from(value: T) -> Self {
        Self {
            inner: Arc::new(BindingInner::from(value)),
        }
    }
}

impl<T> From<T> for BindingInner<T> {
    fn from(value: T) -> Self {
        Self {
            value: RwLock::new(value),
            subscribers: RwLock::new(Vec::new()),
        }
    }
}
