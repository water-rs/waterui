use core::{
    cell::{Ref, RefCell, RefMut},
    fmt::{Debug, Display},
    marker::PhantomData,
    num::NonZeroUsize,
    ops::{Deref, DerefMut},
};

use alloc::{
    boxed::Box,
    rc::{Rc, Weak},
    string::{String, ToString},
};

use crate::{
    subscriber::{BoxSubscriber, FnSubscriber, SubscribeGuard, SubscriberManager},
    Compute, Reactive,
};

// `Binding` is container for two-way data binding.
#[derive(Debug)]
pub struct Binding<T> {
    inner: Rc<BindingInner<T>>,
}

#[derive(Debug)]
pub struct WeakBinding<T> {
    inner: Weak<BindingInner<T>>,
}

#[derive(Debug)]
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

pub struct BindingGuard<'a, T> {
    guard: Ref<'a, T>,
}

pub struct BindingMutGuard<'a, T> {
    guard: Option<RefMut<'a, T>>,
    subscribers: &'a SubscriberManager,
}

impl<T> Drop for BindingMutGuard<'_, T> {
    fn drop(&mut self) {
        drop(self.guard.take());
        self.subscribers.notify();
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

impl<T> Binding<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: Rc::new(BindingInner::new(value)),
        }
    }

    // Create a two-way connection between two bindings.
    pub fn bridge<F1, F2, T2>(&self, to_this: F1, to_new: F2) -> Binding<T2>
    where
        F1: 'static + Fn(&Binding<T2>) -> T,
        F2: 'static + Fn(&Binding<T>) -> T2,
        T: 'static,
        T2: 'static,
    {
        let new = Binding::new(to_new(self));

        let weak_this = self.downgrade();
        let weak_new = new.downgrade();

        let new_guard = new.subscribers().preassign();
        let this_guard = self.subscribers().preassign();

        {
            let weak_new = weak_new.clone();
            let weak_this = weak_this.clone();

            new.subscribers().register_with_id(
                new_guard,
                Box::new(FnSubscriber::new(
                    weak_this,
                    move |weak_this| {
                        if let Some(this) = weak_this.upgrade() {
                            this.set(to_this(&weak_new.upgrade().unwrap()))
                        }
                    },
                    move |weak_this| {
                        weak_this
                            .upgrade()
                            .inspect(|this| this.cancel_subscriber(this_guard));
                    },
                )),
            );
        }

        self.subscribers().register_with_id(
            this_guard,
            Box::new(FnSubscriber::new(
                weak_new,
                move |weak_new| {
                    if let Some(new) = weak_new.upgrade() {
                        new.set(to_new(&weak_this.upgrade().unwrap()))
                    }
                },
                move |weak_new| {
                    weak_new
                        .upgrade()
                        .inspect(|new| new.cancel_subscriber(new_guard));
                },
            )),
        );

        new
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

    pub fn notify(&self) {
        self.inner.subscribers.notify();
    }

    pub fn subscribe(&self, subscriber: impl Fn() + 'static) -> SubscribeGuard<'_, Self> {
        SubscribeGuard::new(self, self.subscribers().register(Box::new(subscriber)))
    }

    pub fn watch(&self, watcher: impl Fn(&Self) + 'static) -> SubscribeGuard<'_, Self>
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

    pub fn to_compute<F, T2>(&self, f: F) -> ToCompute<T, T2, F>
    where
        F: Fn(&Binding<T>) -> T2,
    {
        ToCompute::new(self.clone(), f)
    }

    pub fn display(&self) -> impl Compute<Output = String>
    where
        T: Display,
    {
        self.to_compute(|v| v.get().to_string())
    }

    pub unsafe fn from_raw(ptr: *const T) -> Self {
        Self {
            inner: Rc::from_raw(ptr as *const BindingInner<T>),
        }
    }

    pub fn into_raw(self) -> *const T {
        Rc::into_raw(self.inner) as *const T
    }
}

pub struct ToCompute<T, T2, F> {
    source: Binding<T>,
    f: F,
    _marker: PhantomData<T2>,
}

impl<T, T2, F> ToCompute<T, T2, F> {
    pub fn new(source: Binding<T>, f: F) -> Self {
        Self {
            source,
            f,
            _marker: PhantomData,
        }
    }
}

impl<T, T2, F> Compute for ToCompute<T, T2, F>
where
    F: Fn(&Binding<T>) -> T2,
{
    type Output = T2;

    fn compute(&self) -> Self::Output {
        (self.f)(&self.source)
    }
}

impl<T, T2, F> Reactive for ToCompute<T, T2, F> {
    fn register_subscriber(&self, subscriber: BoxSubscriber) -> Option<NonZeroUsize> {
        self.source.register_subscriber(subscriber)
    }

    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.source.cancel_subscriber(id);
    }
    fn notify(&self) {
        self.source.notify();
    }
}

impl<T: Clone> Compute for Binding<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.get().deref().clone()
    }
}

impl<T> Reactive for Binding<T> {
    fn register_subscriber(&self, subscriber: BoxSubscriber) -> Option<NonZeroUsize> {
        Some(self.inner.subscribers.register(subscriber))
    }
    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.inner.subscribers.cancel(id);
    }
    fn notify(&self) {
        self.inner.subscribers.notify();
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
