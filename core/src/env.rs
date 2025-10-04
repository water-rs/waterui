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
//! ```rust
//! use waterui_core::{Environment,env::use_env};
//!
//! // Create an environment with a string value
//! let env = Environment::new().with(String::from("Hello, world!"));
//!
//! // Access the value in a child view
//! // let view = use_env(|env: &Environment| {
//! //     if let Some(message) = env.get::<String>() {
//! //         message
//! //     } else {
//! //         "No message found".to_string()
//! //     }
//! // });
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
/// use waterui_core::Environment;
///
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
    view::{Hook, ViewConfiguration},
};

/// A type-indexed storage container for values in an environment.
///
/// This struct allows storing values of any type `V` indexed by a type key `K`.
#[derive(Debug)]
pub struct Store<K,V>{
    key:PhantomData<K>,
    value:V,
}

impl Environment {
    /// Creates a new empty environment.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    /// Stores a value in the environment indexed by type `K`.
    ///
    /// # Arguments
    /// * `value` - The value to store
    pub fn store<K: 'static,V: 'static>(mut self,value:V)->Self{
        self.insert(Store{key:PhantomData::<V>,value});
        self
    }

    /// Queries for a value in the environment indexed by type `K`.
    ///
    /// # Returns
    /// An optional reference to the stored value
    #[must_use] 
    pub fn query<K: 'static,V: 'static>(&self) -> Option<&V>{
        self.get::<Store<K,V>>().map(|s|&s.value)
    }

    /// Installs a plugin into the environment.
    ///
    /// Plugins can register values or modifiers that will be available to all views.
    #[must_use]
    pub fn install(mut self, plugin: impl Plugin) -> Self {
        plugin.install(&mut self);
        self
    }

    /// Inserts a value into the environment.
    ///
    /// If a value of the same type already exists, it will be replaced.
    pub fn insert<T: 'static>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Rc::new(value));
    }

    /// Inserts a view configuration hook into the environment.
    ///
    /// Hooks allow you to intercept and modify view configurations globally.
    pub fn insert_hook<T: ViewConfiguration, V: View>(
        &mut self,
        hook: impl Fn(&Self, T) -> V + 'static,
    ) {
        self.insert(Hook::new(hook));
    }

    /// Removes a value from the environment by its type.
    pub fn remove<T: 'static>(&mut self) {
        self.map.remove(&TypeId::of::<T>());
    }

    /// Adds a value to the environment and returns the modified environment.
    ///
    /// This is a fluent interface for chaining multiple additions.
    #[must_use]
    pub fn with<T: 'static>(mut self, value: T) -> Self {
        self.insert(value);
        self
    }

    /// Retrieves a reference to a value from the environment by its type.
    ///
    /// Returns `None` if no value of the requested type exists.
    ///
    /// # Panics
    ///
    /// This function will panic if a value of the requested type exists in the environment,
    /// but the stored value cannot be downcast to the requested type. This should never happen
    /// if only `insert` and `with` are used to add values.
    #[must_use]
    #[allow(clippy::coerce_container_to_any)]
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .map(|v| v.downcast_ref::<T>().unwrap())
    }
}

/// A view that provides access to the environment.
///
/// `UseEnv` allows child views to access values stored in the environment
/// through a handler function.
#[derive(Debug, Clone)]
pub struct UseEnv<V, H> {
    handler: H,
    _marker: PhantomData<V>,
}

impl<V, H> UseEnv<V, H> {
    /// Creates a new `UseEnv` with the provided handler.
    #[must_use]
    pub const fn new(handler: H) -> Self {
        Self {
            handler,
            _marker: PhantomData,
        }
    }
}

/// Creates a view that can access the environment.
///
/// This function takes a closure that receives a reference to the environment
/// and returns a view. It's a convenience wrapper around `UseEnv`.
#[must_use]
pub const fn use_env<P, V, F>(f: F) -> UseEnv<V, IntoHandlerOnce<F, P, V>>
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

/// A view that extends the environment with an additional value.
///
/// `With` wraps a child view and provides an extended environment that
/// includes a new value of type `T`.
#[derive(Debug, Clone)]
pub struct With<V, T> {
    content: V,
    value: T,
}

impl<V, T> With<V, T> {
    /// Creates a new `With` view that wraps the provided content and adds
    /// the given value to the environment for all child views.
    pub const fn new(content: V, value: T) -> Self {
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
