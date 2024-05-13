use core::{
    cell::{Cell, Ref, RefCell, RefMut},
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use alloc::rc::{Rc, Weak};

use crate::{
    reactive::ReactiveExt,
    subscriber::{Metadata, SubscribeGuard, Subscriber, SubscriberId, SubscriberManager},
    Compute, Reactive,
};

/// `Binding` is a reactive and shareable mutable containers, always using for two-way data binding.
#[derive(Debug, Default)]
pub struct Binding<T: 'static> {
    inner: Rc<BindingInner<T>>,
}

pub fn binding<T>(value: T) -> Binding<T> {
    Binding::new(value)
}

// A weak reference of `Binding`.
#[derive(Debug)]
pub struct WeakBinding<T> {
    inner: Weak<BindingInner<T>>,
}

#[derive(Debug, Default)]
struct BindingInner<T> {
    value: RefCell<T>,
    subscribers: SubscriberManager,
}

impl<T> BindingInner<T> {
    pub const fn new(value: T) -> Self {
        Self {
            value: RefCell::new(value),
            subscribers: SubscriberManager::new(),
        }
    }
}

impl<T> WeakBinding<T> {
    pub fn upgrade(&self) -> Option<Binding<T>> {
        Weak::upgrade(&self.inner).map(|inner| Binding { inner })
    }
}

#[derive(Debug)]
pub struct BindingGuard<'a, T> {
    guard: Ref<'a, T>,
}

#[derive(Debug)]
pub struct BindingMutGuard<'a, T> {
    guard: Option<RefMut<'a, T>>,
    subscribers: &'a SubscriberManager,
}

impl<T> Drop for BindingMutGuard<'_, T> {
    fn drop(&mut self) {
        drop(self.guard.take());
        self.subscribers.notify(&Metadata::new());
    }
}

impl<T> Deref for BindingGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.deref()
    }
}

impl<T> Deref for BindingMutGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().unwrap().deref()
    }
}

impl<T> DerefMut for BindingMutGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_mut().unwrap().deref_mut()
    }
}

// Create a two-way connection between two bindings.
pub fn bridge<T1, T2, F1, F2>(b1: Binding<T1>, b2: Binding<T2>, to_b2: F1, to_b1: F2)
where
    F1: 'static + Fn(&T1) -> T2,
    F2: 'static + Fn(&T2) -> T1,
{
    let skip = Rc::new(Cell::new(false));
    let weak_b1 = b1.downgrade();
    let weak_b2 = b2.downgrade();
    let guard1 = {
        let skip = skip.clone();
        let weak_b2 = weak_b2.clone();
        b1.watch({
            move |t1| {
                if !skip.get() {
                    if let Some(b2) = weak_b2.upgrade() {
                        skip.set(!skip.get());

                        b2.set(to_b2(t1.get().deref()));
                    }
                }
            }
        })
        .into_raw()
    }
    .unwrap();

    let guard2 = {
        let weak_b1 = weak_b1.clone();
        b2.watch(move |t2| {
            if !skip.get() {
                if let Some(b1) = weak_b1.upgrade() {
                    skip.set(!skip.get());

                    b1.set(to_b1(t2.get().deref()));
                }
            }
        })
        .into_raw()
    }
    .unwrap();

    b2.when_drop(move || {
        if let Some(b1) = weak_b1.upgrade() {
            b1.cancel_subscriber(guard1);
        }
    });

    b1.when_drop(move || {
        if let Some(b2) = weak_b2.upgrade() {
            b2.cancel_subscriber(guard2);
        }
    });
}

pub struct WhenDrop<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for WhenDrop<F> {
    fn drop(&mut self) {
        (self.0.take().unwrap())();
    }
}
impl<T> Binding<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: Rc::new(BindingInner::new(value)),
        }
    }

    pub fn get(&self) -> BindingGuard<'_, T> {
        BindingGuard {
            guard: self.inner.value.borrow(),
        }
    }

    pub fn get_mut(&self) -> BindingMutGuard<'_, T> {
        BindingMutGuard {
            guard: Some(self.inner.value.borrow_mut()),
            subscribers: &self.inner.subscribers,
        }
    }

    pub fn when_drop(&self, f: impl FnOnce() + 'static) {
        let wrapper = WhenDrop(Some(f));
        self.subscribe(move || {
            let _ = &wrapper;
        })
        .leak();
    }

    pub fn inspect(&self, f: impl FnOnce(&T)) {
        f(self.get().deref());
    }

    pub fn inspect_mut(&self, f: impl FnOnce(&mut T)) {
        f(self.get_mut().deref_mut());
    }

    pub fn downgrade(&self) -> WeakBinding<T> {
        WeakBinding {
            inner: Rc::downgrade(&self.inner),
        }
    }

    pub fn set(&self, value: impl Into<T>) {
        self.inspect_mut(move |v| *v = value.into());
    }

    pub fn notify(&self, metadata: &Metadata) {
        self.inner.subscribers.notify(metadata);
    }

    pub fn watch(&self, watcher: impl Fn(&Self) + 'static) -> SubscribeGuard<&Self>
    where
        T: 'static,
    {
        let this = self.downgrade();
        self.subscribe(move || {
            watcher(&this.upgrade().unwrap());
        })
    }

    pub fn subscribers(&self) -> &SubscriberManager {
        &self.inner.subscribers
    }

    /// # Safety
    ///  The pointer must have been obtained through `Binding::into_raw`, and the inner `Rc` is valid.
    pub unsafe fn from_raw(ptr: *const T) -> Self {
        Self {
            inner: Rc::from_raw(ptr as *const BindingInner<T>),
        }
    }

    pub fn into_raw(self) -> *const T {
        Rc::into_raw(self.inner) as *const T
    }
}

impl<T: Clone> Compute for Binding<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.get().deref().clone()
    }
}

impl<T> Reactive for Binding<T> {
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<SubscriberId> {
        Some(self.inner.subscribers.register(subscriber))
    }
    fn cancel_subscriber(&self, id: SubscriberId) {
        self.inner.subscribers.cancel(id);
    }
}

impl<T> Clone for Binding<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Clone for WeakBinding<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}
