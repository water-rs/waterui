use core::{any::type_name, fmt::Debug, num::NonZeroUsize};

use alloc::rc::{Rc, Weak};

use crate::{Compute, Subscriber};
mod bridge;
mod container;
mod f;

pub struct Binding<T> {
    inner: Rc<dyn BindingImpl<T>>,
}

pub struct WeakBinding<T> {
    inner: Weak<dyn BindingImpl<T>>,
}

pub trait BindingImpl<T>: Compute<Output = T> {
    fn set(&self, value: T);
}

impl<T> WeakBinding<T> {
    pub fn upgrade(&self) -> Option<Binding<T>> {
        Weak::upgrade(&self.inner).map(|inner| Binding { inner })
    }
}

impl<T: Clone + 'static> Binding<T> {
    pub fn new(value: impl Into<T>) -> Self {
        Self::constant(value.into())
    }
    pub fn constant(value: T) -> Self {
        Self {
            inner: Rc::new(container::ContainerBinding::new(value)),
        }
    }
}
impl<T> Binding<T> {
    pub fn from_impl(value: impl BindingImpl<T> + 'static) -> Self {
        Self {
            inner: Rc::new(value),
        }
    }
    pub fn from_fn(getter: impl 'static + Fn() -> T, setter: impl 'static + Fn(T)) -> Self {
        Self::from_impl(f::FnBinding::new(getter, setter))
    }

    pub fn from_fn_with_state<State: 'static>(
        state: State,
        getter: impl 'static + Fn(&State) -> T,
        setter: impl 'static + Fn(&State, T),
    ) -> Self {
        Self::from_impl(f::FnBindingWithState::new(state, getter, setter))
    }

    pub fn notify(&self) {
        self.inner.notify();
    }

    pub fn bridge<T2, From, To>(&self, from_new: From, to_new: To) -> Binding<T2>
    where
        From: 'static + Fn(T2) -> T,
        To: 'static + Fn(T) -> T2,
        T: 'static,
        T2: 'static,
    {
        Binding::from_impl(bridge::BirdgeBinding::new(self.clone(), from_new, to_new))
    }

    pub fn bridge_with_state<State, T2, From, To>(
        &self,
        state: State,
        from_new: From,
        to_new: To,
    ) -> Binding<T2>
    where
        State: 'static,
        From: 'static + Fn(&State, T2) -> T,
        To: 'static + Fn(&State, T) -> T2,
        T: 'static,
        T2: 'static,
    {
        Binding::from_impl(bridge::BirdgeBindingWithState::new(
            state,
            self.clone(),
            from_new,
            to_new,
        ))
    }

    pub fn downgrade(&self) -> WeakBinding<T> {
        WeakBinding {
            inner: Rc::downgrade(&self.inner),
        }
    }

    pub fn set(&self, value: impl Into<T>) {
        self.inner.set(value.into());
    }
}

impl<T> Compute for Binding<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.inner.compute()
    }
    fn register_subscriber(&self, subscriber: Subscriber) -> Option<NonZeroUsize> {
        self.inner.register_subscriber(subscriber)
    }
    fn cancel_subscriber(&self, id: NonZeroUsize) {
        self.inner.cancel_subscriber(id)
    }
    fn notify(&self) {
        self.inner.notify()
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

impl<T> Debug for Binding<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<T> Debug for WeakBinding<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}
