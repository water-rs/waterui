use core::cell::RefCell;

use alloc::rc::Rc;

use crate::{
    watcher::{SharedWatcherManager, Watcher, WatcherGuard, WatcherManager},
    Compute,
};

trait BindingImpl<T> {
    fn get(&self) -> T;
    fn set(&self, value: T);
}

pub struct Binding<T: 'static> {
    inner: Rc<dyn BindingImpl<T>>,
    watchers: SharedWatcherManager<T>,
}

impl<T: Clone> From<T> for Binding<T> {
    fn from(value: T) -> Self {
        Self::from_impl(Container::new(value))
    }
}

impl<T: Clone> Binding<T> {
    pub fn new(value: impl Into<T>) -> Self {
        Self::from(value.into())
    }
}

impl<T> Binding<T> {
    fn from_impl(source: impl BindingImpl<T> + 'static) -> Self {
        Self {
            inner: Rc::new(source),
            watchers: Rc::new(WatcherManager::new()),
        }
    }

    pub fn get(&self) -> T {
        self.inner.get()
    }

    pub fn set(&self, value: T) {
        self.inner.set(value);
        self.watchers.notify(|| self.get());
    }

    pub fn from_fn(getter: impl 'static + Fn() -> T, setter: impl Fn(T) + 'static) -> Self {
        Self::from_impl(Custom::new(getter, setter))
    }
}

impl<T> Clone for Binding<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            watchers: self.watchers.clone(),
        }
    }
}

struct Container<T>(RefCell<T>);

impl<T> Container<T> {
    pub fn new(value: T) -> Self {
        Self(RefCell::new(value))
    }
}

struct Custom<Getter, Setter> {
    getter: Getter,
    setter: Setter,
}

impl<Getter, Setter> Custom<Getter, Setter> {
    pub fn new(getter: Getter, setter: Setter) -> Self {
        Self { getter, setter }
    }
}

impl<T: Clone + 'static> BindingImpl<T> for Container<T> {
    fn get(&self) -> T {
        self.0.borrow().clone()
    }
    fn set(&self, value: T) {
        self.0.replace(value);
    }
}

impl<T, Getter, Setter> BindingImpl<T> for Custom<Getter, Setter>
where
    Getter: Fn() -> T,
    Setter: Fn(T),
{
    fn get(&self) -> T {
        (self.getter)()
    }
    fn set(&self, value: T) {
        (self.setter)(value);
    }
}

impl<T> Compute for Binding<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.get()
    }

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        WatcherGuard::from_id(&self.watchers, self.watchers.register(watcher))
    }
}
