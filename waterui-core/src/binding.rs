use std::{
    any::type_name,
    fmt::{Debug, Display},
    hash::Hash,
    ops::{Deref, DerefMut},
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

#[derive(Default)]
pub struct Binding<T: ?Sized> {
    inner: Arc<RawBinding<T>>,
}

impl<T: PartialEq + ?Sized> PartialEq for Binding<T> {
    fn eq(&self, other: &Self) -> bool {
        let left = self.inner.value.read().unwrap();
        let right = other.inner.value.read().unwrap();
        left.deref() == right.deref()
    }
}

impl<T: Eq + ?Sized> Eq for Binding<T> {}

impl<T: PartialEq> PartialEq<T> for Binding<T> {
    fn eq(&self, other: &T) -> bool {
        let left = self.get();
        left.deref() == other
    }
}

impl<T: Hash> Hash for Binding<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.get().hash(state)
    }
}

impl<A, T: Extend<A>> Extend<A> for Binding<T> {
    fn extend<Iter: IntoIterator<Item = A>>(&mut self, iter: Iter) {
        self.get_mut().extend(iter)
    }
}

impl<T> Debug for Binding<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<T: Display> Display for Binding<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().fmt(f)
    }
}

impl<T: Iterator> Iterator for Binding<T> {
    type Item = T::Item;
    fn next(&mut self) -> Option<Self::Item> {
        self.get_mut().next()
    }
}

impl<T> Clone for Binding<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[derive(Default)]
struct RawBinding<T: ?Sized> {
    subscribers: RwLock<Vec<SubscriberObject>>,
    value: RwLock<T>,
}

#[repr(C)]
#[derive(Debug)]
pub struct SubscriberObject {
    state: *const (),
    subscriber: unsafe extern "C" fn(*const ()),
}

impl SubscriberObject {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(),
    {
        let boxed: Box<dyn Fn()> = Box::new(f);
        let state = Box::into_raw(boxed) as *const ();
        extern "C" fn from_fn_impl(state: *const ()) {
            let boxed = state as *const Box<dyn Fn()>;
            unsafe {
                let f = &*boxed;
                (f)()
            }
        }
        Self::from_raw(state, from_fn_impl)
    }

    pub fn call(&self) {
        unsafe { (self.subscriber)(self.state) }
    }

    fn from_raw(state: *const (), subscriber: unsafe extern "C" fn(*const ())) -> Self {
        Self { state, subscriber }
    }
}

impl<T> RawBinding<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: RwLock::new(value),
            subscribers: RwLock::new(Vec::new()),
        }
    }

    pub fn get(&self) -> RwLockReadGuard<T> {
        self.value.read().unwrap()
    }

    pub fn get_mut(&self) -> MutGuard<T> {
        MutGuard {
            guard: self.value.write().unwrap(),
            subscribers: self.subscribers.read().unwrap(),
        }
    }

    pub fn make_effect(&self) {
        for subscriber in self.subscribers.read().unwrap().deref() {
            subscriber.call();
        }
    }
}

impl<T> Binding<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: Arc::new(RawBinding::new(value)),
        }
    }

    pub fn get(&self) -> RwLockReadGuard<T> {
        self.inner.get()
    }

    pub fn get_mut(&self) -> MutGuard<T> {
        self.inner.get_mut()
    }

    pub fn set(&self, value: T) {
        *self.get_mut() = value;
    }

    pub fn make_effect(&self) {
        self.inner.make_effect();
    }

    pub fn add_subscriber(&self, subscriber: SubscriberObject) {
        self.inner.subscribers.write().unwrap().push(subscriber);
    }

    pub fn watch<F>(&self, watcher: F)
    where
        F: Fn(&T),
    {
        let weak = Arc::downgrade(&self.inner);
        let subscriber = SubscriberObject::new(move || {
            let value = weak.upgrade().unwrap();
            let value = value.get();
            (watcher)(&value)
        });
        self.add_subscriber(subscriber);
    }
}

impl Binding<String> {
    pub fn string(string: impl Into<String>) -> Self {
        Self::new(string.into())
    }
}

pub struct MutGuard<'a, T> {
    guard: RwLockWriteGuard<'a, T>,
    subscribers: RwLockReadGuard<'a, Vec<SubscriberObject>>,
}

impl<'a, T> Deref for MutGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.deref()
    }
}

impl<'a, T> DerefMut for MutGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.deref_mut()
    }
}

impl<'a, T> Drop for MutGuard<'a, T> {
    fn drop(&mut self) {
        for subscriber in self.subscribers.deref() {
            subscriber.call();
        }
    }
}

impl Binding<bool> {
    pub fn toggle(&self) {
        let mut guard = self.get_mut();
        let new_value = !*guard.deref();
        *guard.deref_mut() = new_value
    }
}

macro_rules! impl_num {
    ($($ty:ty),*) => {
        $(
            impl Binding<$ty> {
                pub fn increase(&self, num: $ty) {
                    *self.get_mut() += num;
                }

                pub fn decrease(&self, num: $ty) {
                    *self.get_mut() -= num;
                }
            }
        )*
    };
}

impl_num!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);
