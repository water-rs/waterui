use core::{
    any::{type_name, Any, TypeId},
    fmt::Debug,
    future::Future,
    marker::PhantomData,
    pin::Pin,
};

use alloc::{boxed::Box, collections::BTreeMap, rc::Rc, vec, vec::Vec};
use async_executor::LocalExecutor;

pub trait Executor {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()>>>);
}

impl Debug for dyn Executor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

#[derive(Debug, Clone)]
pub struct Environment {
    executor: Rc<dyn Executor>,
    layers: Vec<Rc<EnvironmentLayer>>,
}

#[cfg(feature = "default-executor")]
impl Default for Environment {
    fn default() -> Self {
        Self::new(LocalExecutor::new())
    }
}

impl Executor for async_executor::LocalExecutor<'_> {
    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()>>>) {
        self.spawn(future).detach();
    }
}

impl EnvironmentLayer {
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .map(|v| v.downcast_ref().unwrap())
    }

    pub fn insert<T: 'static>(&mut self, value: T) {
        let mut map = BTreeMap::new();
        map.insert(TypeId::of::<T>(), value);
    }
}

#[derive(Debug)]
struct EnvironmentLayer {
    map: BTreeMap<TypeId, Box<dyn Any>>,
}

use crate::View;

impl Environment {
    pub fn new(executor: impl Executor + 'static) -> Self {
        Self {
            executor: Rc::new(executor),
            layers: vec![Rc::new(EnvironmentLayer::new())],
        }
    }
    pub fn insert<T: 'static>(&mut self, value: T) {
        let layer = self.layers.last_mut().unwrap();
        if let Some(layer) = Rc::get_mut(layer) {
            layer.map.insert(TypeId::of::<T>(), Box::new(value));
        } else {
            let mut layer = EnvironmentLayer::new();
            layer.insert(value);
            self.layers.push(Rc::new(layer));
        }
    }

    pub fn with<T: 'static>(mut self, value: T) -> Self {
        self.insert(value);
        self
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        for layer in self.layers.iter().rev() {
            if let Some(value) = layer.get() {
                return Some(value);
            }
        }
        None
    }

    pub fn task<Fut>(&self, fut: Fut)
    where
        Fut: Future<Output = ()> + 'static,
    {
        self.executor.spawn(Box::pin(fut))
    }
}

pub struct UseEnv<V, F> {
    f: F,
    _marker: PhantomData<V>,
}

impl<V, F> UseEnv<V, F> {
    pub fn new(f: F) -> Self {
        Self {
            f,
            _marker: PhantomData,
        }
    }
}

pub fn use_env<V, F>(f: F) -> UseEnv<V, F>
where
    V: View,
    F: 'static + Fn(&Environment) -> V,
{
    UseEnv::new(f)
}

impl<V, F> View for UseEnv<V, F>
where
    V: View,
    F: 'static + Fn(&Environment) -> V,
{
    fn body(self, env: &Environment) -> impl View {
        (self.f)(env)
    }
}
