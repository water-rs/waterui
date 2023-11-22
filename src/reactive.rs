use std::{
    fmt::{Arguments, Display},
    ops::{Add, Deref, DerefMut},
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

pub struct Ref<T: 'static> {
    inner: Arc<RawRef<T>>,
}

impl<T: 'static> Clone for Ref<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

pub trait IntoRef<T> {
    fn into_ref(self) -> Ref<T>;
}

impl<T> IntoRef<T> for T {
    fn into_ref(self) -> Ref<T> {
        Ref::once(self)
    }
}

impl<T> IntoRef<T> for &Ref<T> {
    fn into_ref(self) -> Ref<T> {
        self.clone()
    }
}

impl<T: Display> Display for Ref<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.get().deref().fmt(f)
    }
}

impl<T> Deref for Ref<T> {
    type Target = RawRef<T>;
    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

struct Once<T> {
    value: RwLock<Option<T>>,
}

impl<T> Once<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: RwLock::new(Some(value)),
        }
    }
}

impl<T> Updater<T> for Once<T> {
    fn update(&self) -> T {
        self.value
            .write()
            .unwrap()
            .take()
            .expect("This value cannot be updated")
    }
}

impl<T> Ref<T> {
    pub fn new(value: T) -> Self
    where
        T: Clone,
    {
        Self::new_with_updater(move || value.clone())
    }
    pub fn once(value: T) -> Self
    where
        T: 'static,
    {
        Self::new_with_updater(Once::new(value))
    }

    pub fn new_with_updater(updater: impl Updater<T> + 'static) -> Self {
        Self::from_raw(RawRef::new(updater))
    }

    pub fn from_raw(raw: RawRef<T>) -> Self {
        Self {
            inner: Arc::new(raw),
        }
    }
}

impl<T: 'static> From<T> for Ref<T> {
    fn from(value: T) -> Self {
        Self::once(value)
    }
}
impl Ref<String> {
    pub fn string(value: impl Into<String>) -> Self {
        Self::new(value.into())
    }
}

pub struct RefGuard<'a, T: 'static> {
    guard: RwLockReadGuard<'a, Option<T>>,
}

pub struct MutRefGuard<'a, T: 'static> {
    guard: Option<RwLockWriteGuard<'a, Option<T>>>,
    r: &'a RawRef<T>,
}

impl<'a, T> Deref for RefGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().unwrap()
    }
}

impl<'a, T> Deref for MutRefGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().unwrap().as_ref().unwrap()
    }
}

impl<'a, T> DerefMut for MutRefGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_mut().unwrap().as_mut().unwrap()
    }
}

impl<'a, T> Drop for MutRefGuard<'a, T> {
    fn drop(&mut self) {
        drop(self.guard.take().unwrap());
        self.r.call_watcher();
    }
}

pub trait Watcher<T> {
    fn call_watcher(&self, r: &RawRef<T>);
}

impl<F, T> Watcher<T> for F
where
    F: Fn(&RawRef<T>),
{
    fn call_watcher(&self, r: &RawRef<T>) {
        (self)(r)
    }
}

pub struct RawRef<T: 'static> {
    value: RwLock<Option<T>>,
    watchers: RwLock<Vec<Box<dyn Watcher<T>>>>,
    updater: BoxUpdater<T>,
}

pub trait Updater<T> {
    fn update(&self) -> T;
}

impl<F, T> Updater<T> for F
where
    F: Fn() -> T,
{
    fn update(&self) -> T {
        (self)()
    }
}

type BoxUpdater<T> = Box<dyn Updater<T>>;

impl<T> RawRef<T> {
    pub fn new(updater: impl Updater<T> + 'static) -> Self {
        Self {
            value: RwLock::new(None),
            watchers: RwLock::new(Vec::new()),
            updater: Box::new(updater),
        }
    }

    pub fn watch(&self, watcher: impl Watcher<T> + 'static) {
        self.watchers.write().unwrap().push(Box::new(watcher));
    }

    pub fn subcribe<R>(&self, r: Ref<R>) {
        self.watch(move |_: &_| {
            r.need_update();
        });
    }

    pub fn call_watcher(&self) {
        let _ = self
            .watchers
            .read()
            .unwrap()
            .iter()
            .map(|watcher| watcher.call_watcher(self));
    }

    pub fn need_update(&self) {
        *self.value.write().unwrap() = None;
    }

    pub fn update(&self) {
        // Prevent unnecessary computation
        if self.value.read().unwrap().is_none() {
            *self.value.write().unwrap() = Some(self.updater.update());
        }
    }

    pub fn get(&self) -> RefGuard<T> {
        self.update();
        RefGuard {
            guard: self.value.read().unwrap(),
        }
    }

    pub fn get_mut(&self) -> MutRefGuard<T> {
        self.update();
        MutRefGuard {
            guard: Some(self.value.write().unwrap()),
            r: &self,
        }
    }

    pub fn set(&self, value: impl Into<T>) {
        *self.get_mut() = value.into();
    }

    pub fn into_inner(&self) -> T {
        self.update();
        self.value.write().unwrap().take().unwrap()
    }
}

macro_rules! impl_str_reactive_left {
    ($(($ty1:ty,$ty2:ty)),*) => {
        $(
            impl Add<$ty2> for Ref<$ty1> {
                type Output = Ref<String>;
                fn add(self, rhs: $ty2) -> Self::Output {
                    let left = self.clone();
                    let r = Ref::new_with_updater(move || {
                        let mut left:String = left.get().clone().into();
                        left.push_str(rhs.as_ref());
                        left
                    });
                    self.subcribe(r.clone());
                    r
                }
            }
        )*
    };
}

impl_str_reactive_left!(
    (&'static str, &'static str),
    (&'static str, String),
    (String, &'static str),
    (String, String)
);

macro_rules! impl_str_reactive_right {
    ($(($ty1:ty,$ty2:ty)),*) => {
        $(
            impl Add<Ref<$ty2>> for $ty1 {
                type Output = Ref<String>;
                fn add(self, rhs: Ref<$ty2>) -> Self::Output {
                    let right=rhs.clone();
                    let r = Ref::new_with_updater(move || {
                        let mut left:String = self.to_string();
                        left.push_str(right.get().as_ref());
                        left
                    });
                    rhs.subcribe(r.clone());
                    r
                }
            }
        )*
    };
}

impl_str_reactive_right!(
    (&'static str, &'static str),
    (&'static str, String),
    (String, &'static str),
    (String, String)
);

macro_rules! impl_str_reactive {
    ($(($ty1:ty,$ty2:ty)),*) => {
        $(
            impl Add<Ref<$ty2>> for Ref<$ty1> {
                type Output = Ref<String>;
                fn add(self, rhs: Ref<$ty2>) -> Self::Output {
                    let left=self.clone();
                    let right=rhs.clone();
                    let r = Ref::new_with_updater(move || {
                        let mut left:String=left.get().deref().to_string();
                        left.push_str(right.get().deref());
                        left
                    });
                    self.subcribe(r.clone());

                    rhs.subcribe(r.clone());
                    r
                }
            }
        )*
    };
}

impl_str_reactive!(
    (&'static str, &'static str),
    (&'static str, String),
    (String, &'static str),
    (String, String)
);

pub fn reactive<T: 'static>(value: T) -> Ref<T> {
    Ref::once(value)
}

#[test]
fn test() {
    let s1 = reactive("Swift");
    let s = "Hello," + s1.clone();
    s1.set("Rust");

    println!("{s}");
}
