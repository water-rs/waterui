use std::{
    fmt::{Debug, Display},
    hash::Hash,
    ops::{Deref, DerefMut},
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

pub struct Reactive<T: 'static> {
    inner: Arc<RawReactive<T>>,
}

impl From<&str> for Reactive<String> {
    fn from(value: &str) -> Self {
        Self::from(value.to_string())
    }
}

impl<T: Clone> From<&[T]> for Reactive<Vec<T>> {
    fn from(value: &[T]) -> Self {
        Self::from(value.to_vec())
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

impl<T> From<T> for Reactive<T> {
    fn from(value: T) -> Self {
        Self::new(value)
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
    pub fn new(value: impl Into<T>) -> Self {
        Self {
            inner: Arc::new(RawReactive::new(value.into())),
        }
    }

    pub fn get(&self) -> ReactiveGuard<T> {
        self.inner.get()
    }

    pub fn get_mut(&self) -> MutReactiveGuard<T> {
        self.inner.get_mut()
    }

    pub fn set(&self, value: impl Into<T>) {
        *self.get_mut() = value.into();
    }

    pub fn to<T2>(&self, f: impl 'static + Fn(&T) -> T2) -> Reactive<T2> {
        let binding = self.clone();
        let reactive = Reactive::<T2>::new_with_updater(move || f(binding.get().deref()));
        let weak = Arc::downgrade(&reactive.inner);
        self.inner.add_subcriber(move || {
            if let Some(weak) = weak.upgrade() {
                weak.need_update()
            }
        });
        reactive
    }

    pub fn new_with_updater<Output: Into<T>>(updater: impl 'static + Fn() -> Output) -> Self {
        Self::from_raw(RawReactive::new_with_updater(move || (updater)().into()))
    }

    fn from_raw(raw: RawReactive<T>) -> Self {
        Self {
            inner: Arc::new(raw),
        }
    }
}

impl Reactive<bool> {
    pub fn toggle(&self) {
        let mut guard = self.get_mut();
        *guard = !*guard;
    }
}

pub struct ReactiveGuard<'a, T> {
    guard: RwLockReadGuard<'a, Option<T>>,
}

pub struct MutReactiveGuard<'a, T> {
    guard: RwLockWriteGuard<'a, Option<T>>,
    subscribers: &'a RwLock<Vec<Subscriber>>,
}

impl<'a, T> Deref for ReactiveGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().unwrap()
    }
}

impl<'a, T> Deref for MutReactiveGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().unwrap()
    }
}

impl<'a, T> DerefMut for MutReactiveGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_mut().unwrap()
    }
}

impl<'a, T> Drop for MutReactiveGuard<'a, T> {
    fn drop(&mut self) {
        for subsriber in self.subscribers.write().unwrap().deref() {
            subsriber()
        }
    }
}

type Updater<T> = Box<dyn Fn() -> T>;
type Subscriber = Box<dyn Fn()>;

struct RawReactive<T> {
    value: RwLock<Option<T>>,
    subscribers: RwLock<Vec<Subscriber>>,
    updater: Updater<T>,
}

impl<T> RawReactive<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: RwLock::new(Some(value)),
            subscribers: RwLock::new(Vec::new()),
            updater: Box::new(|| {
                panic!("This Reactive have no dependency and should not be updated")
            }),
        }
    }

    pub fn new_with_updater(updater: impl 'static + Fn() -> T) -> Self {
        Self {
            value: RwLock::new(None),
            subscribers: RwLock::new(Vec::new()),
            updater: Box::new(updater),
        }
    }

    pub fn add_subcriber(&self, subscriber: impl Fn() + 'static) {
        self.subscribers.write().unwrap().push(Box::new(subscriber))
    }

    pub fn need_update(&self) {
        *self.value.write().unwrap() = None;
    }

    pub fn get(&self) -> ReactiveGuard<T> {
        let guard = loop {
            let guard = self.value.read().unwrap();
            if guard.is_none() {
                drop(guard);
                *self.value.write().unwrap() = Some((self.updater)());
            } else {
                break guard;
            }
        };

        ReactiveGuard { guard }
    }

    pub fn get_mut(&self) -> MutReactiveGuard<T> {
        let mut guard = self.value.write().unwrap();
        guard.get_or_insert_with(|| (self.updater)());
        MutReactiveGuard {
            guard,
            subscribers: &self.subscribers,
        }
    }
}
