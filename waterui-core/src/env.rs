use core::{
    any::{type_name, Any, TypeId},
    fmt::Debug,
    marker::PhantomData,
};

use alloc::{collections::BTreeMap, rc::Rc};

#[derive(Debug, Clone, Default)]
pub struct Environment {
    map: BTreeMap<TypeId, Rc<dyn Any>>,
}

use crate::View;

impl Environment {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn plugin(mut self, plugin: impl Plugin) -> Self {
        plugin.install(&mut self);
        self
    }

    pub fn insert<T: 'static>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Rc::new(value));
    }

    pub fn remove<T: 'static>(&mut self) {
        self.map.remove(&TypeId::of::<T>());
    }

    pub fn with<T: 'static>(mut self, value: T) -> Self {
        self.insert(value);
        self
    }

    pub fn get<T: 'static>(&self) -> &T {
        self.try_get()
            .unwrap_or_else(|| panic!("Environment value `{}` not found", type_name::<T>()))
    }

    pub fn try_get<T: 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .map(|v| v.downcast_ref::<T>().unwrap())
    }
}

pub trait Plugin: Sized + 'static {
    fn install(self, env: &mut Environment) {
        env.insert(self);
    }
    fn uninstall(self, env: &mut Environment) {
        env.remove::<Self>()
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
    F: 'static + FnOnce(Environment) -> V,
{
    UseEnv::new(f)
}

impl<V, F> View for UseEnv<V, F>
where
    V: View,
    F: 'static + FnOnce(Environment) -> V,
{
    fn body(self, env: Environment) -> impl View {
        (self.f)(env)
    }
}
