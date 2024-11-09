use core::{
    any::{Any, TypeId},
    fmt::Debug,
    marker::PhantomData,
};

use alloc::{collections::BTreeMap, rc::Rc};

#[derive(Debug, Clone, Default)]
pub struct Environment {
    map: BTreeMap<TypeId, Rc<dyn Any>>,
}

use crate::{
    components::Metadata,
    handler::{HandlerFnOnce, HandlerOnce, IntoHandlerOnce},
    View,
};

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

    pub fn get<T: 'static>(&self) -> Option<&T> {
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

pub struct UseEnv<V, H> {
    handler: H,
    _marker: PhantomData<V>,
}

impl<V, H> UseEnv<V, H> {
    pub fn new(handler: H) -> Self {
        Self {
            handler,
            _marker: PhantomData,
        }
    }
}

pub fn use_env<P, V, F>(f: F) -> UseEnv<V, IntoHandlerOnce<F, P, V>>
where
    V: View,
    F: HandlerFnOnce<P, V>,
{
    UseEnv::new(IntoHandlerOnce::new(f))
}

impl<V, H> View for UseEnv<V, H>
where
    V: View,
    H: HandlerOnce<V>,
{
    fn body(self, env: &Environment) -> impl View {
        self.handler.handle(env)
    }
}

pub struct With<V, T> {
    content: V,
    value: T,
}

impl<V, T> With<V, T> {
    pub fn new(content: V, value: T) -> Self {
        Self { content, value }
    }
}

impl<V, T> View for With<V, T>
where
    V: View,
    T: 'static,
{
    fn body(self, env: &Environment) -> impl View {
        let env = env.clone().with(self.value);
        Metadata::new(self.content, env)
    }
}
