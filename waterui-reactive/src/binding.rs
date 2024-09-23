use core::{
    any::type_name,
    cell::RefCell,
    fmt::Debug,
    marker::PhantomData,
    ops::{Deref, RangeBounds},
};

use alloc::{boxed::Box, rc::Rc};

use crate::{
    watcher::{SharedWatcherManager, Watcher, WatcherGuard},
    Compute,
};

pub trait CustomBinding<T>: Compute<Output = T> {
    fn set(&self, value: T);
}

trait BindingImpl<T> {
    fn get(&self) -> T;
    fn set(&self, value: T);
    fn add_watcher(&self, watcher: Watcher<T>) -> WatcherGuard;
    fn cloned(&self) -> Binding<T>;
}

impl<B: CustomBinding<T>, T> CustomBinding<T> for &B {
    fn set(&self, value: T) {
        CustomBinding::set(*self, value);
    }
}

impl<B, T> BindingImpl<T> for B
where
    B: CustomBinding<T> + 'static,
{
    fn get(&self) -> T {
        self.compute()
    }

    fn set(&self, value: T) {
        CustomBinding::set(self, value);
    }
    fn add_watcher(&self, watcher: Watcher<T>) -> WatcherGuard {
        Compute::add_watcher(self, watcher)
    }
    fn cloned(&self) -> Binding<T> {
        Binding(Box::new(self.clone()))
    }
}
pub struct Binding<T>(Box<dyn BindingImpl<T>>);

impl<T> Debug for Binding<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<T: 'static + Clone> Binding<T> {
    pub fn container(value: T) -> Self {
        Self::custom(Container::new(value))
    }
}

impl<T: Default + Clone + 'static> Default for Binding<T> {
    fn default() -> Self {
        Self::container(T::default())
    }
}

pub fn binding<T: 'static + Clone>(value: T) -> Binding<T> {
    Binding::container(value)
}

impl<T> Binding<T> {
    pub fn custom(custom: impl CustomBinding<T> + 'static) -> Self {
        Self(Box::new(custom))
    }

    pub fn get(&self) -> T {
        self.0.get()
    }

    pub fn set(&self, value: T) {
        self.0.set(value);
    }

    pub fn map<Output, Getter, Setter>(&self, getter: Getter, setter: Setter) -> Binding<Output>
    where
        T: 'static,
        Output: 'static,
        Getter: 'static + Fn(T) -> Output,
        Setter: 'static + Fn(&Binding<T>, Output),
    {
        Binding::custom(Map {
            binding: self.clone(),
            getter: Rc::new(getter),
            setter: Rc::new(setter),
            _marker: PhantomData,
        })
    }

    pub fn filter(&self, filter: impl 'static + Fn(&T) -> bool) -> Self
    where
        T: 'static,
    {
        self.map(
            |value| value,
            move |binding, value| {
                if filter(&value) {
                    binding.set(value);
                }
            },
        )
    }
}

impl<T: PartialOrd + 'static> Binding<T> {
    pub fn range(self, range: impl RangeBounds<T> + 'static) -> Self {
        self.filter(move |value| range.contains(value))
    }
}
impl Binding<i32> {
    pub fn int(i: i32) -> Self {
        Self::container(i)
    }
}

impl Binding<bool> {
    pub fn bool(value: bool) -> Self {
        Self::container(value)
    }

    pub fn toggle(&self) {
        self.set(!self.get())
    }
}

impl<T> Clone for Binding<T> {
    fn clone(&self) -> Self {
        self.0.cloned()
    }
}

#[derive(Debug, Clone)]
struct Container<T: Clone> {
    value: RefCell<T>,
    watchers: SharedWatcherManager<T>,
}

impl<T: Clone> Container<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: RefCell::new(value),
            watchers: SharedWatcherManager::default(),
        }
    }
}

impl<T: Clone + 'static> Compute for Container<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.value.borrow().deref().clone()
    }

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        WatcherGuard::from_id(&self.watchers, self.watchers.register(watcher))
    }
}

impl<T: Clone + 'static> CustomBinding<T> for Container<T> {
    fn set(&self, value: T) {
        self.value.replace(value.clone());
        self.watchers.notify(move || value.clone());
    }
}

impl<T> Compute for Binding<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.get()
    }

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        self.0.add_watcher(watcher)
    }
}

struct Map<Input, Output, Getter, Setter> {
    binding: Binding<Input>,
    getter: Rc<Getter>,
    setter: Rc<Setter>,
    _marker: PhantomData<Output>,
}

impl<Input, Output, Getter, Setter> Clone for Map<Input, Output, Getter, Setter> {
    fn clone(&self) -> Self {
        Self {
            binding: self.binding.clone(),
            getter: self.getter.clone(),
            setter: self.setter.clone(),
            _marker: PhantomData,
        }
    }
}

impl<Input, Output, Getter, Setter> Compute for Map<Input, Output, Getter, Setter>
where
    Output: 'static,
    Getter: 'static + Fn(Input) -> Output,
{
    type Output = Output;
    fn compute(&self) -> Self::Output {
        (self.getter)(self.binding.compute())
    }
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        let getter = self.getter.clone();
        self.binding
            .add_watcher(Watcher::new(move |value, metadata| {
                watcher.notify_with_metadata(getter(value), metadata)
            }))
    }
}

impl<Input, Output, Getter, Setter> CustomBinding<Output> for Map<Input, Output, Getter, Setter>
where
    Output: 'static,

    Getter: 'static + Fn(Input) -> Output,
    Setter: Fn(&Binding<Input>, Output),
{
    fn set(&self, value: Output) {
        (self.setter)(&self.binding, value)
    }
}
