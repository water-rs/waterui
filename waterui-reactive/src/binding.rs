use core::{
    cell::{Ref, RefCell, RefMut},
    fmt::{Debug, Display},
    mem::replace,
    num::NonZeroUsize,
    ops::{AddAssign, Deref, DerefMut, SubAssign},
};

use alloc::{borrow::Cow, boxed::Box, rc::Rc};

use crate::{subscriber::SubscriberManager, Subscriber};

pub struct Binding<T> {
    inner: Rc<BindingInner<T>>,
}

pub type BindingStr = Binding<Cow<'static, str>>;
pub type BindingBool = Binding<bool>;
pub type BindingInt = Binding<isize>;

impl<T: Debug> Debug for Binding<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.read().fmt(f)
    }
}

impl<T: Display> Display for Binding<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.read().fmt(f)
    }
}

impl<T> Clone for Binding<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Binding<Option<T>> {
    pub fn take(&self) -> Option<T> {
        let mut result = None;
        self.peek_mut(|v| result = v.take());
        result
    }
}

impl<T: Clone> Binding<T> {
    pub fn get(&self) -> T {
        self.read().clone()
    }
}

impl<T> Binding<T> {
    pub fn new(value: impl Into<T>) -> Self {
        Self::constant(value.into())
    }

    pub fn constant(value: T) -> Self {
        Self {
            inner: Rc::new(BindingInner {
                value: RefCell::new(value),
                subscribers: SubscriberManager::new(),
            }),
        }
    }

    pub fn bridge<F, Output>(&self, f: F) -> Binding<Output>
    where
        F: 'static + Fn(&T) -> Output,
        Output: 'static,
        T: 'static,
    {
        let result = Binding::new(f(self.read().deref()));

        self.register_subscriber({
            let result = result.clone();
            let source = self.clone();

            Box::new(move || result.set(f(source.read().deref())))
        });
        result
    }

    pub fn replace(&self, value: T) -> T {
        let result = replace(self.write().deref_mut(), value);
        self.notify();
        result
    }

    pub fn set(&self, value: impl Into<T>) {
        let _ = self.replace(value.into());
    }

    pub fn action<Env>(&self, action: impl Fn(&Self, Env)) -> impl Fn(Env) {
        let binding = self.clone();
        move |env| {
            action(&binding, env);
        }
    }

    fn read(&self) -> Ref<'_, T> {
        self.inner.value.borrow()
    }

    fn write(&self) -> RefMut<'_, T> {
        self.inner.value.borrow_mut()
    }

    fn notify(&self) {
        self.inner.subscribers.notify();
    }

    pub fn peek(&self, f: impl FnOnce(&T)) {
        f(self.read().deref());
    }

    pub fn peek_mut(&self, f: impl FnOnce(&mut T)) {
        f(self.write().deref_mut());
        self.notify();
    }

    pub fn register_subscriber(&self, subscriber: Subscriber) -> NonZeroUsize {
        self.inner.subscribers.register(subscriber)
    }

    pub fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.inner.subscribers.cancel(id);
    }

    pub fn into_raw(self) -> *const T {
        Rc::into_raw(self.inner) as *const T
    }

    /// # Safety
    ///
    /// This function is unsafe because improper use may lead to
    /// memory problems. For example, a double-free may occur if the
    /// function is called twice on the same raw pointer.
    pub unsafe fn from_raw(raw: *const T) -> Self {
        Self {
            inner: Rc::from_raw(raw as *const BindingInner<T>),
        }
    }
}

impl<T: AddAssign> Binding<T> {
    pub fn add(&self, n: T) {
        self.peek_mut(move |v| {
            v.add_assign(n);
        });
    }
}

impl<T: SubAssign> Binding<T> {
    pub fn sub(&self, n: T) {
        self.peek_mut(move |v| {
            v.sub_assign(n);
        });
    }
}

struct BindingInner<T> {
    subscribers: SubscriberManager,
    value: RefCell<T>,
}
