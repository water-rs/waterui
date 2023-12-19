use std::{
    fmt::{Debug, Display},
    hash::Hash,
    ops::{Deref, DerefMut},
    sync::Arc,
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard, Weak},
};

use crate::AttributedString;

pub struct Reactive<T: 'static> {
    inner: Arc<ReactiveInner<T>>,
}

impl<T: Default> Default for Reactive<T> {
    fn default() -> Self {
        Self::new_with_updater(|| T::default())
    }
}

impl<T: Clone + Send + Sync> From<T> for Reactive<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

pub trait IntoReactive<T> {
    fn into_reactive(self) -> Reactive<T>;
}

impl<T1, T2> IntoReactive<T2> for Reactive<T1>
where
    T1: Send + Sync,
    for<'a> &'a T1: Into<T2>,
    T2: Send + Sync,
{
    fn into_reactive(self) -> Reactive<T2> {
        self.to(|v| v.into())
    }
}

impl IntoReactive<String> for &str {
    fn into_reactive(self) -> Reactive<String> {
        Reactive::new(self.into())
    }
}

impl<T: Clone + Send + Sync> IntoReactive<Vec<T>> for &[T] {
    fn into_reactive(self) -> Reactive<Vec<T>> {
        Reactive::new(self.into())
    }
}

impl IntoReactive<AttributedString> for &str {
    fn into_reactive(self) -> Reactive<AttributedString> {
        Reactive::new(self.into())
    }
}

impl IntoReactive<AttributedString> for String {
    fn into_reactive(self) -> Reactive<AttributedString> {
        Reactive::new(self.into())
    }
}

impl<T> IntoReactive<T> for &Reactive<T> {
    fn into_reactive(self) -> Reactive<T> {
        self.clone()
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
    pub fn new(value: T) -> Self
    where
        T: Send + Sync,
    {
        Self::from_inner(ReactiveInner::new(value))
    }

    pub fn new_with_updater(updater: impl 'static + Send + Sync + Fn() -> T) -> Self {
        Self::from_inner(ReactiveInner::new_with_updater(updater))
    }

    fn from_inner(inner: ReactiveInner<T>) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn get(&self) -> ReactiveGuard<T> {
        self.inner.get()
    }

    pub(crate) unsafe fn from_raw(ptr: *const ReactiveInner<T>) -> Self {
        unsafe {
            Self {
                inner: Arc::from_raw(ptr),
            }
        }
    }

    pub(crate) fn into_raw(self) -> *const ReactiveInner<T> {
        Arc::into_raw(self.inner)
    }

    pub fn get_mut(&self) -> MutReactiveGuard<T> {
        self.inner.get_mut()
    }

    pub fn set(&self, value: impl Into<T>) {
        self.inner.set(value.into())
    }

    pub fn add_subcriber(&self, subscriber: Subscriber) {
        self.inner.add_subcriber(subscriber)
    }

    pub fn to<T2>(&self, f: impl 'static + Send + Sync + Fn(&T) -> T2) -> Reactive<T2>
    where
        T: Send + Sync,
        T2: Send + Sync,
    {
        let original = self.clone();

        let output = Reactive::<T2>::new_with_updater(move || f(original.get().deref()));

        output.depend(self);

        output
    }

    fn depend<T2>(&self, dependency: &Reactive<T2>) {
        let original = Arc::downgrade(&self.inner);

        extern "C" fn depend_subscriber<T: 'static>(data: *mut ()) {
            let data = data as *const Weak<ReactiveInner<T>>;

            unsafe {
                if let Some(reactive) = (*data).upgrade() {
                    reactive.need_update()
                }
            }
        }
        unsafe {
            dependency.add_subcriber(Subscriber::from_raw(
                Box::into_raw(Box::new(original)) as *mut (),
                depend_subscriber::<T>,
            ))
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
    guard: Option<RwLockWriteGuard<'a, Option<T>>>,
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
        self.guard.as_ref().unwrap().deref().as_ref().unwrap()
    }
}

impl<'a, T> DerefMut for MutReactiveGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_mut().unwrap().deref_mut().as_mut().unwrap()
    }
}

impl<'a, T> Drop for MutReactiveGuard<'a, T> {
    fn drop(&mut self) {
        drop(self.guard.take().unwrap());
        for subsriber in self.subscribers.read().unwrap().deref() {
            subsriber.call()
        }
    }
}

type Updater<T> = Box<dyn Send + Sync + Fn() -> T>;

#[repr(C)]
#[derive(Debug)]
pub struct Subscriber {
    state: *mut (),
    subscriber: unsafe extern "C" fn(*mut ()),
}

unsafe impl Send for Subscriber {}
unsafe impl Sync for Subscriber {}

impl Drop for Subscriber {
    fn drop(&mut self) {
        unsafe { drop(Box::from_raw(self.state)) }
    }
}

impl Subscriber {
    pub fn new<F>(f: F) -> Self
    where
        F: FnMut() + Send + Sync,
    {
        let boxed: Box<Box<dyn FnMut()>> = Box::new(Box::new(f));
        let state = Box::into_raw(boxed) as *mut ();
        extern "C" fn from_fn_impl(state: *mut ()) {
            let boxed = state as *mut Box<dyn Fn()>;
            unsafe {
                let f = &*boxed;
                (f)()
            }
        }
        unsafe { Self::from_raw(state, from_fn_impl) }
    }

    pub fn call(&self) {
        unsafe { (self.subscriber)(self.state) }
    }

    unsafe fn from_raw(state: *mut (), subscriber: unsafe extern "C" fn(*mut ())) -> Self {
        Self { state, subscriber }
    }
}

pub(crate) struct ReactiveInner<T: 'static> {
    value: RwLock<Option<T>>,
    subscribers: RwLock<Vec<Subscriber>>,
    updater: Updater<T>,
}

impl<T> ReactiveInner<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: RwLock::new(Some(value)),
            subscribers: RwLock::new(Vec::new()),
            updater: Box::new(|| panic!("This Reactive is created by `new`, cannot be updated")),
        }
    }
    pub fn new_with_updater(updater: impl 'static + Send + Sync + Fn() -> T) -> Self {
        Self {
            value: RwLock::new(None),
            subscribers: RwLock::new(Vec::new()),
            updater: Box::new(updater),
        }
    }

    pub fn add_subcriber(&self, subscriber: Subscriber) {
        self.subscribers.write().unwrap().push(subscriber);
    }

    pub fn need_update(&self) {
        self.value.write().unwrap().take();
        self.make_effect();
    }

    pub fn make_effect(&self) {
        for subscriber in self.subscribers.read().unwrap().deref() {
            subscriber.call()
        }
    }

    fn update(&self) {
        let mut guard = self.value.write().unwrap();

        if guard.is_none() {
            *(guard.deref_mut()) = Some((self.updater)())
        }
    }

    pub fn get(&self) -> ReactiveGuard<T> {
        let guard = loop {
            let guard = self.value.read().unwrap();
            if guard.is_some() {
                break guard;
            } else {
                drop(guard);
                self.update();
            }
        };

        ReactiveGuard { guard }
    }

    pub fn get_mut(&self) -> MutReactiveGuard<T> {
        let mut guard = self.value.write().unwrap();

        if guard.is_none() {
            *(guard.deref_mut()) = Some((self.updater)())
        }

        MutReactiveGuard {
            guard: Some(guard),
            subscribers: &self.subscribers,
        }
    }

    pub fn set(&self, value: T) {
        *self.value.write().unwrap() = Some(value);
        for subscriber in self.subscribers.read().unwrap().deref() {
            subscriber.call()
        }
    }
}
