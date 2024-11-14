use core::{
    any::{type_name, Any},
    cell::RefCell,
    fmt::Debug,
    marker::PhantomData,
    ops::{Add, AddAssign, Deref, DerefMut, RangeBounds},
};

use alloc::{boxed::Box, rc::Rc};
use waterui_str::Str;

use crate::{
    compute::ComputeResult,
    watcher::{Watcher, WatcherGuard, WatcherManager},
    zip::FlattenMap,
    Compute, ComputeExt, Computed,
};

pub trait CustomBinding: Compute {
    fn set(&self, value: Self::Output);
}

pub struct Binding<T: ComputeResult>(Box<dyn BindingImpl<Output = T>>);

trait BindingImpl {
    type Output: ComputeResult;
    fn get(&self) -> Self::Output;
    fn set(&self, value: Self::Output);
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard;
    fn cloned(&self) -> Binding<Self::Output>;
}

impl<T: CustomBinding + Clone + 'static> BindingImpl for T {
    type Output = T::Output;
    fn get(&self) -> Self::Output {
        self.compute()
    }
    fn set(&self, value: Self::Output) {
        <T as CustomBinding>::set(self, value)
    }
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        <T as Compute>::add_watcher(self, watcher)
    }
    fn cloned(&self) -> Binding<Self::Output> {
        Binding::custom(self.clone())
    }
}

impl<T: ComputeResult> Debug for Binding<T> {
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

    pub fn append(&self, value: impl AsRef<str>) {
        self.handle(|v| {
            *v += value;
        });
    }

    pub fn clear(&self) {
        self.set(Str::new());
    }
}

impl<T: 'static + Add + ComputeResult> Add for Binding<T>
where
    T::Output: ComputeResult,
{
    type Output = Computed<T::Output>;
    fn add(self, rhs: Self) -> Self::Output {
        self.zip(rhs)
            .flatten_map(|left, right| left + right)
            .computed()
    }
}

impl<T: 'static + Add + ComputeResult> Add<T> for Binding<T>
where
    T::Output: ComputeResult,
{
    type Output = Computed<T::Output>;
    fn add(self, rhs: T) -> Self::Output {
        ComputeExt::map(self, move |this| this + rhs.clone()).computed()
    }
}

pub struct BindingMutGuard<'a, T: ComputeResult> {
    binding: &'a Binding<T>,
    value: Option<T>,
}

impl<'a, T: ComputeResult> BindingMutGuard<'a, T> {
    pub fn new(binding: &'a Binding<T>) -> Self {
        Self {
            value: Some(binding.get()),
            binding,
        }
    }
}

impl<'a, T: ComputeResult> Deref for BindingMutGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.value.as_ref().unwrap()
    }
}

impl<'a, T: ComputeResult> DerefMut for BindingMutGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value.as_mut().unwrap()
    }
}

impl<'a, T: ComputeResult> Drop for BindingMutGuard<'a, T> {
    fn drop(&mut self) {
        self.binding.set(self.value.take().unwrap());
    }
}

impl Add<&'static str> for Binding<Str> {
    type Output = Computed<Str>;

    fn add(self, rhs: &'static str) -> Self::Output {
        ComputeExt::map(self, move |this| this + rhs).computed()
    }
}

impl<T: ComputeResult> Binding<T> {
    pub fn custom(custom: impl CustomBinding<Output = T> + Clone + 'static) -> Self {
        Self(Box::new(custom))
    }

    pub fn get(&self) -> T {
        self.0.get()
    }

    pub(crate) fn as_container(&self) -> Option<&Container<T>> {
        let any = &self.0 as &dyn Any;
        any.downcast_ref()
    }

    pub fn get_mut(&self) -> BindingMutGuard<T> {
        BindingMutGuard::new(self)
    }

    pub fn handle(&self, handler: impl FnOnce(&mut T)) {
        if let Some(container) = self.as_container() {
            {
                let mut value = container.value.borrow_mut();
                handler(&mut value);
            }
            container.watchers.notify(self.get());
        } else {
            let mut temp = self.get();

            handler(&mut temp);
            self.set(temp);
        }
    }

    pub fn set(&self, value: T) {
        if self.get() != value {
            self.0.set(value);
        }
    }

    pub fn mapping<Output, Getter, Setter>(
        source: &Self,
        getter: Getter,
        setter: Setter,
    ) -> Binding<Output>
    where
        Output: ComputeResult,
        Getter: 'static + Fn(T) -> Output,
        Setter: 'static + Fn(&Binding<T>, Output),
    {
        Binding::custom(Mapping {
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
        Binding::mapping(
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
        self.handle(|v| *v += n);
    }

    pub fn decrement(&self, n: i32) {
        self.handle(|v| *v -= n);
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

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        WatcherGuard::from_id(&self.watchers, self.watchers.register(watcher))
    }
}

impl<T: ComputeResult> CustomBinding for Container<T> {
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

    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        self.0.add_watcher(watcher)
    }
}

struct Mapping<Input: ComputeResult, Output, Getter, Setter> {
    binding: Binding<Input>,
    getter: Rc<Getter>,
    setter: Rc<Setter>,
    _marker: PhantomData<Output>,
}

impl<Input, Output, Getter, Setter> Clone for Mapping<Input, Output, Getter, Setter>
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

impl<Input, Output, Getter, Setter> Compute for Mapping<Input, Output, Getter, Setter>
where
    Input: ComputeResult,
    Output: ComputeResult,
    Getter: 'static + Fn(Input) -> Output,
    Setter: 'static,
{
    type Output = Output;
    fn compute(&self) -> Self::Output {
        (self.getter)(self.binding.compute())
    }
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard {
        let getter = self.getter.clone();
        self.binding.watch(Watcher::new(move |value, metadata| {
            watcher.notify_with_metadata(getter(value), metadata)
        }))
    }
}

impl<Input, Output, Getter, Setter> CustomBinding for Mapping<Input, Output, Getter, Setter>
where
    Input: ComputeResult,
    Output: ComputeResult,

    Getter: 'static + Fn(Input) -> Output,
    Setter: 'static + Fn(&Binding<Input>, Output),
{
    fn set(&self, value: Output) {
        (self.setter)(&self.binding, value)
    }
}
