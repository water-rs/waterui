use core::{
    any::{type_name, Any},
    cell::{RefCell, RefMut},
    fmt::Debug,
    marker::PhantomData,
    ops::{Add, AddAssign, Deref, DerefMut, RangeBounds},
};

use alloc::{boxed::Box, rc::Rc};
use waterui_str::Str;

use crate::{
    compute::ComputeResult,
    watcher::{Watcher, WatcherGuard, WatcherManager},
    Compute, ComputeExt, Computed,
};

pub trait CustomBinding<T>: Compute<Output = T> {
    fn set(&self, value: T);
}

trait BindingImpl<T: ComputeResult>: 'static {
    fn get(&self) -> T;
    fn set(&self, value: T);
    fn watch(&self, watcher: Watcher<T>) -> WatcherGuard;
    fn as_any(&self) -> &dyn Any;
    fn cloned(&self) -> Binding<T>;
}

impl<B, T> BindingImpl<T> for B
where
    B: CustomBinding<T> + 'static,
    T: ComputeResult,
{
    fn get(&self) -> T {
        self.compute()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn set(&self, value: T) {
        CustomBinding::set(self, value);
    }
    fn watch(&self, watcher: Watcher<T>) -> WatcherGuard {
        Compute::watch(self, watcher)
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

impl<T: ComputeResult> Binding<T> {
    pub fn container(value: T) -> Self {
        Self::custom(Container::new(value))
    }
}

impl<T: Default + ComputeResult> Default for Binding<T> {
    fn default() -> Self {
        Self::container(T::default())
    }
}

pub fn binding<T: ComputeResult>(value: T) -> Binding<T> {
    Binding::container(value)
}

impl Binding<Str> {
    pub fn str(s: impl Into<Str>) -> Self {
        Self::container(s.into())
    }
}

impl<T: 'static + Add + ComputeResult> Add for Binding<T>
where
    T::Output: ComputeResult,
{
    type Output = Computed<T::Output>;
    fn add(self, rhs: Self) -> Self::Output {
        (self, rhs).map(|(left, right)| left + right).computed()
    }
}

impl<T: 'static + Add + ComputeResult> Add<T> for Binding<T>
where
    T::Output: ComputeResult,
{
    type Output = Computed<T::Output>;
    fn add(self, rhs: T) -> Self::Output {
        ComputeExt::map(&self, move |this| this + rhs.clone()).computed()
    }
}

pub enum BindingMutGuard<'a, T: ComputeResult> {
    Container {
        container: &'a Container<T>,
        ref_mut: Option<RefMut<'a, T>>,
    },
    Other {
        binding: &'a Binding<T>,
        value: Option<T>,
    },
}

impl<'a, T: ComputeResult> BindingMutGuard<'a, T> {
    pub fn new(binding: &'a Binding<T>) -> Self {
        if let Some(container) = binding.as_container() {
            Self::Container {
                ref_mut: Some(container.value.borrow_mut()),
                container,
            }
        } else {
            Self::Other {
                value: Some(binding.get()),
                binding,
            }
        }
    }
}

impl<'a, T: ComputeResult> Deref for BindingMutGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        match self {
            BindingMutGuard::Container { ref_mut, .. } => ref_mut.as_deref().unwrap(),
            BindingMutGuard::Other { value, .. } => value.as_ref().unwrap(),
        }
    }
}

impl<'a, T: ComputeResult> DerefMut for BindingMutGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            BindingMutGuard::Container { ref_mut, .. } => ref_mut.as_deref_mut().unwrap(),
            BindingMutGuard::Other { value, .. } => value.as_mut().unwrap(),
        }
    }
}

impl<'a, T: ComputeResult> Drop for BindingMutGuard<'a, T> {
    fn drop(&mut self) {
        match self {
            BindingMutGuard::Container { container, ref_mut } => {
                ref_mut.take().unwrap();
                container.notify();
            }
            BindingMutGuard::Other { binding, value } => {
                binding.set(value.take().unwrap());
            }
        }
    }
}

impl Add<&'static str> for Binding<Str> {
    type Output = Computed<Str>;

    fn add(self, rhs: &'static str) -> Self::Output {
        ComputeExt::map(&self, move |this| this + rhs).computed()
    }
}

impl<T: ComputeResult> Binding<T> {
    pub fn custom(custom: impl CustomBinding<T> + 'static) -> Self {
        Self(Box::new(custom))
    }

    pub fn get(&self) -> T {
        self.0.get()
    }

    pub(crate) fn as_container(&self) -> Option<&Container<T>> {
        self.0.as_any().downcast_ref()
    }

    pub fn get_mut(&self) -> BindingMutGuard<T> {
        BindingMutGuard::new(self)
    }

    pub fn handle(&self, handler: impl FnOnce(&mut T)) {
        let mut temp = self.get();

        handler(&mut temp);
        self.set(temp);
    }

    pub fn set(&self, value: T) {
        if self.0.get() != value {
            self.0.set(value);
        }
    }

    pub fn map<Output, Getter, Setter>(
        source: &Self,
        getter: Getter,
        setter: Setter,
    ) -> Binding<Output>
    where
        Output: ComputeResult,
        Getter: 'static + Fn(T) -> Output,
        Setter: 'static + Fn(&Binding<T>, Output),
    {
        Binding::custom(Map {
            binding: source.clone(),
            getter: Rc::new(getter),
            setter: Rc::new(setter),
            _marker: PhantomData,
        })
    }

    pub fn filter(&self, filter: impl 'static + Fn(&T) -> bool) -> Self
    where
        T: 'static,
    {
        Binding::map(
            self,
            |value| value.clone(),
            move |binding, value| {
                if filter(&value) {
                    binding.set(value);
                }
            },
        )
    }
}

impl<T: PartialOrd + ComputeResult> Binding<T> {
    pub fn range(self, range: impl RangeBounds<T> + 'static) -> Self {
        self.filter(move |value| range.contains(value))
    }
}

impl Binding<i32> {
    pub fn int(i: i32) -> Self {
        Self::container(i)
    }

    pub fn increment(&self, n: i32) {
        *self.get_mut() += n;
    }

    pub fn decrement(&self, n: i32) {
        *self.get_mut() -= n;
    }
}

impl Binding<bool> {
    pub fn bool(value: bool) -> Self {
        Self::container(value)
    }

    pub fn toggle(&self) {
        self.handle(|v| *v = !*v);
    }
}

impl<T, R> AddAssign<R> for Binding<T>
where
    T: AddAssign<R> + ComputeResult,
{
    fn add_assign(&mut self, rhs: R) {
        self.handle(|v| {
            *v += rhs;
        });
    }
}

impl<T: ComputeResult> Clone for Binding<T> {
    fn clone(&self) -> Self {
        self.0.cloned()
    }
}

#[derive(Debug, Clone)]
pub struct Container<T> {
    value: Rc<RefCell<T>>,
    watchers: WatcherManager<T>,
}

impl<T: ComputeResult> Container<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: Rc::new(RefCell::new(value)),
            watchers: WatcherManager::default(),
        }
    }

    pub fn notify(&self) {
        self.watchers.notify(self.value.borrow().clone());
    }
}

impl<T: ComputeResult> Compute for Container<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.value.borrow().deref().clone()
    }

    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard {
        WatcherGuard::from_id(&self.watchers, self.watchers.register(watcher.into()))
    }
}

impl<T: ComputeResult> CustomBinding<T> for Container<T> {
    fn set(&self, value: T) {
        self.value.replace(value.clone());
        self.watchers.notify(value);
    }
}

impl<T: ComputeResult> Compute for Binding<T> {
    type Output = T;
    fn compute(&self) -> Self::Output {
        self.get()
    }

    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard {
        self.0.watch(watcher.into())
    }
}

struct Map<Input, Output, Getter, Setter> {
    binding: Binding<Input>,
    getter: Rc<Getter>,
    setter: Rc<Setter>,
    _marker: PhantomData<Output>,
}

impl<Input, Output, Getter, Setter> Clone for Map<Input, Output, Getter, Setter>
where
    Input: ComputeResult,
{
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
    Input: ComputeResult,
    Output: ComputeResult,
    Getter: 'static + Fn(Input) -> Output,
{
    type Output = Output;
    fn compute(&self) -> Self::Output {
        (self.getter)(self.binding.compute())
    }
    fn watch(&self, watcher: impl Into<Watcher<Self::Output>>) -> WatcherGuard {
        let getter = self.getter.clone();
        let watcher = watcher.into();
        self.binding.watch(Watcher::new(move |value, metadata| {
            watcher.notify_with_metadata(getter(value), metadata)
        }))
    }
}

impl<Input, Output, Getter, Setter> CustomBinding<Output> for Map<Input, Output, Getter, Setter>
where
    Input: ComputeResult,
    Output: ComputeResult,

    Getter: 'static + Fn(Input) -> Output,
    Setter: Fn(&Binding<Input>, Output),
{
    fn set(&self, value: Output) {
        (self.setter)(&self.binding, value)
    }
}
