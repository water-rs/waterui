//! Environment management module for sharing data across views.
//!
//! This module provides functionality for creating and managing environment contexts
//! that can be passed through the view hierarchy. The environment is a type-based
//! key-value store where types serve as unique keys.
//!
//! The main components are:
//! - `Environment`: A store for typed values that can be passed between views
//! - `UseEnv`: A view that allows consuming environment values
//! - `With`: A view that extends the environment with additional values
//!
//! # Example
//!
//! ```
//! // Create an environment with a string value
//! let env = Environment::new().with(String::from("Hello, world!"));
//!
//! // Access the value in a child view
//! let view = use_env(|env: &Environment| {
//!     if let Some(message) = env.get::<String>() {
//!         Text::new(message)
//!     } else {
//!         Text::new("No message found")
//!     }
//! });
//! ```

use core::{
    any::{Any, TypeId},
    fmt::Debug,
    marker::PhantomData,
};

use alloc::{collections::BTreeMap, rc::Rc};

/// An `Environment` stores a map of types to values.
///
/// Each type can have at most one value in the environment. The environment
/// is used to pass contextual information from parent views to child views.
///
/// # Examples
///
/// ```
/// let mut env = Environment::new();
/// env.insert(String::from("hello"));
///
/// // Get the value back
/// assert_eq!(env.get::<String>(), Some(&String::from("hello")));
///
/// // Remove the value
/// env.remove::<String>();
/// assert_eq!(env.get::<String>(), None);
/// ```
#[derive(Debug, Clone, Default)]
pub struct Environment {
    map: BTreeMap<TypeId, Rc<dyn Any>>,
}

use crate::{
    View,
    components::Metadata,
    handler::{HandlerFnOnce, HandlerOnce, IntoHandlerOnce},
    plugin::Plugin,
    view::{ConfigurableView, Modifier},
};

impl Environment {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install(mut self, plugin: impl Plugin) -> Self {
        plugin.install(&mut self);
        self
    }

    pub fn insert<T: 'static>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Rc::new(value));
    }

    pub fn insert_modifier<V: ConfigurableView>(&mut self, modifier: Modifier<V>) {
        self.insert(modifier);
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
