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

use alloc::{collections::BTreeMap, rc::Rc, vec::Vec};

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
#[derive(Debug, Clone)]
pub struct Environment {
    state: Rc<EnvironmentState>,
}

#[derive(Debug, Clone)]
enum EnvironmentState {
    Map(BTreeMap<TypeId, Rc<dyn Any>>),
    Overlay {
        parent: Rc<Self>,
        key: TypeId,
        entry: EnvironmentEntry,
    },
}

#[derive(Debug, Clone)]
enum EnvironmentEntry {
    Present(Rc<dyn Any>),
    Removed,
}

impl MetadataKey for Environment {}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}

use crate::{
    View,
    components::Metadata,
    extract::Extractor,
    metadata::MetadataKey,
    plugin::Plugin,
    view::{Hook, ViewConfiguration},
};

/// A type-indexed storage container for values in an environment.
///
/// This struct allows storing values of any type `V` indexed by a type key `K`.
#[derive(Debug)]
pub struct Store<K, V> {
    key: PhantomData<K>,
    value: V,
}

impl<K, V> Store<K, V> {
    /// Creates a new store with the given value.
    #[must_use]
    pub const fn new(value: V) -> Self {
        Self {
            key: PhantomData,
            value,
        }
    }

    /// Returns a reference to the stored value.
    #[must_use]
    pub const fn value(&self) -> &V {
        &self.value
    }
}

impl Environment {
    fn insert_any(&mut self, key: TypeId, value: Rc<dyn Any>) {
        match Rc::get_mut(&mut self.state) {
            Some(EnvironmentState::Map(map)) => {
                map.insert(key, value);
            }
            Some(EnvironmentState::Overlay {
                key: overlay_key,
                entry,
                ..
            }) if *overlay_key == key => {
                *entry = EnvironmentEntry::Present(value);
            }
            _ => self.push_overlay(key, EnvironmentEntry::Present(value)),
        }
    }

    fn lookup_any_in_state(state: &EnvironmentState, key: TypeId) -> Option<&Rc<dyn Any>> {
        match state {
            EnvironmentState::Map(map) => map.get(&key),
            EnvironmentState::Overlay {
                parent,
                key: overlay_key,
                entry,
            } => {
                if *overlay_key == key {
                    match entry {
                        EnvironmentEntry::Present(value) => Some(value),
                        EnvironmentEntry::Removed => None,
                    }
                } else {
                    Self::lookup_any_in_state(parent.as_ref(), key)
                }
            }
        }
    }

    fn lookup_any(&self, key: TypeId) -> Option<&Rc<dyn Any>> {
        Self::lookup_any_in_state(self.state.as_ref(), key)
    }

    fn push_overlay(&mut self, key: TypeId, entry: EnvironmentEntry) {
        self.state = Rc::new(EnvironmentState::Overlay {
            parent: self.state.clone(),
            key,
            entry,
        });
    }

    fn extend_from_state(&mut self, state: &EnvironmentState) {
        match state {
            EnvironmentState::Map(map) => {
                for (key, value) in map {
                    self.insert_any(*key, value.clone());
                }
            }
            EnvironmentState::Overlay { parent, key, entry } => {
                self.extend_from_state(parent.as_ref());
                self.push_overlay(*key, entry.clone());
            }
        }
    }

    fn collect_matches_in_state<'a, T: 'static>(
        state: &'a EnvironmentState,
        matches: &mut Vec<&'a T>,
    ) {
        match state {
            EnvironmentState::Map(map) => {
                if let Some(value) = map.get(&TypeId::of::<T>()) {
                    matches.push(
                        value
                            .downcast_ref::<T>()
                            .expect("failed to downcast value while collecting environment state"),
                    );
                }
            }
            EnvironmentState::Overlay { parent, key, entry } => {
                Self::collect_matches_in_state(parent.as_ref(), matches);
                if *key != TypeId::of::<T>() {
                    return;
                }
                match entry {
                    EnvironmentEntry::Present(value) => matches.push(
                        value
                            .downcast_ref::<T>()
                            .expect("failed to downcast value while collecting environment state"),
                    ),
                    EnvironmentEntry::Removed => matches.clear(),
                }
            }
        }
    }

    /// Creates a new empty environment.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Rc::new(EnvironmentState::Map(BTreeMap::new())),
        }
    }

    /// Stores a value in the environment indexed by type `K`.
    ///
    /// # Arguments
    /// * `value` - The value to store
    #[must_use]
    pub fn store<K: 'static, V: 'static>(mut self, value: V) -> Self {
        self.insert(Store {
            key: PhantomData::<K>,
            value,
        });
        self
    }

    /// Queries for a value in the environment indexed by type `K`.
    ///
    /// # Returns
    /// An optional reference to the stored value
    #[must_use]
    pub fn query<K: 'static, V: 'static>(&self) -> Option<&V> {
        self.get::<Store<K, V>>().map(|s| &s.value)
    }

    /// Installs a plugin into the environment.
    ///
    /// Plugins can register values or modifiers that will be available to all views.
    pub fn install(&mut self, plugin: impl Plugin) -> &mut Self {
        plugin.install(self);
        self
    }

    /// Inserts a value into the environment.
    ///
    /// If a value of the same type already exists, it will be replaced.
    pub fn insert<T: 'static>(&mut self, value: T) {
        let key = TypeId::of::<T>();
        let value = Rc::new(value) as Rc<dyn Any>;
        self.insert_any(key, value);
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
        let key = TypeId::of::<T>();
        match Rc::get_mut(&mut self.state) {
            Some(EnvironmentState::Map(map)) => {
                map.remove(&key);
            }
            Some(EnvironmentState::Overlay {
                key: overlay_key,
                entry,
                ..
            }) if *overlay_key == key => {
                *entry = EnvironmentEntry::Removed;
            }
            _ => self.push_overlay(key, EnvironmentEntry::Removed),
        }
    }

    /// Adds a value to the environment and returns the modified environment.
    ///
    /// This is a fluent interface for chaining multiple additions.
    pub fn with<T: 'static>(&mut self, value: T) -> &mut Self {
        self.insert(value);
        self
    }

    /// Returns a new environment that overlays a value on top of the current state.
    ///
    /// This is an O(1) operation backed by structural sharing and avoids copying
    /// the underlying map.
    #[must_use]
    pub fn extending<T: 'static>(&self, value: T) -> Self {
        Self {
            state: Rc::new(EnvironmentState::Overlay {
                parent: self.state.clone(),
                key: TypeId::of::<T>(),
                entry: EnvironmentEntry::Present(Rc::new(value) as Rc<dyn Any>),
            }),
        }
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
        self.lookup_any(TypeId::of::<T>())
            .map(|v| v.downcast_ref::<T>().expect("failed to downcast value"))
    }

    /// Retrieves the `index`-th visible value of type `T` from oldest to newest.
    ///
    /// This is primarily used by action extractors to support repeated
    /// same-typed [`crate::extract::State`] values in a single handler.
    #[must_use]
    pub fn get_nth<T: 'static>(&self, index: usize) -> Option<&T> {
        let mut matches = Vec::new();
        Self::collect_matches_in_state(self.state.as_ref(), &mut matches);
        matches.into_iter().nth(index)
    }

    /// Retrieves a reference to a value from the environment by its type,
    /// inserting a new value if it does not already exist.
    ///
    /// The new value is created by calling the provided closure `f`.
    #[must_use]
    pub fn get_or_insert_with<T: 'static, F: FnOnce() -> T>(&mut self, f: F) -> &T {
        if self.lookup_any(TypeId::of::<T>()).is_none() {
            self.insert(f());
        }
        self.get::<T>()
            .expect("value missing from environment after insertion")
    }

    /// Extracts a value from the environment using the `Extractor` trait.
    ///
    /// This is a convenience method for extracting values that implement `Extractor`.
    ///
    /// # Errors
    ///
    /// Returns an error if extraction fails (e.g., value not found).
    pub fn extract<T: Extractor>(&self) -> Result<T, anyhow::Error> {
        T::extract(self)
    }

    /// Replays this environment's overlays on top of `parent`.
    ///
    /// This preserves repeated same-typed overlay entries instead of collapsing
    /// them into a single visible value.
    #[must_use]
    pub fn layered_on(&self, parent: &Self) -> Self {
        let mut layered = parent.clone();
        layered.extend_from_state(self.state.as_ref());
        layered
    }
}

/// A view that provides access to the environment.
///
/// `UseEnv` allows child views to access values stored in the environment
/// through a handler function.
#[derive(Debug, Clone)]
pub struct UseEnv<F> {
    handler: F,
}

impl<F> UseEnv<F> {
    /// Creates a new `UseEnv` with the provided handler.
    #[must_use]
    pub const fn new(handler: F) -> Self {
        Self { handler }
    }
}

/// Creates a view that can access the environment.
///
/// This function takes a closure that receives a value extracted from the environment
/// and returns a view. The closure parameter type must implement `Extractor`.
///
/// # Example
///
/// ```rust,ignore
/// use waterui_core::env::use_env;
///
/// // Extract a specific type from environment
/// let view = use_env(|theme: Theme| {
///     text!("Current theme: {}", theme.name())
/// });
///
/// // Extract multiple types using a tuple
/// let view = use_env(|(nav, db): (NavigationController, Database)| {
///     // ...
/// });
/// ```
#[must_use]
pub fn use_env<E, V, F>(f: F) -> UseEnv<impl FnOnce(&Environment) -> V>
where
    E: Extractor,
    V: View,
    F: FnOnce(E) -> V + 'static,
{
    UseEnv::new(move |env: &Environment| {
        let extracted = E::extract(env).expect("failed to extract value from environment");
        f(extracted)
    })
}

impl<V, F> View for UseEnv<F>
where
    V: View,
    F: FnOnce(&Environment) -> V + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        (self.handler)(env)
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

impl<V: View, T: 'static> With<V, T> {
    /// Creates a new `With` view that wraps the provided content and adds
    /// the given value to the environment for all child views.
    pub const fn new(content: V, value: T) -> Self {
        Self { content, value }
    }
}

/// Wraps a view and provides an extended environment value.
pub const fn with<V: View, T: 'static>(view: V, value: T) -> With<V, T> {
    With::new(view, value)
}

impl<V: View, T: 'static> View for With<V, T> {
    fn body(self, env: &Environment) -> impl View {
        let env = env.extending(self.value);
        Metadata::new(self.content, env)
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use super::*;

    #[test]
    fn extending_reuses_parent_state_via_overlay() {
        let mut base = Environment::new();
        base.insert(7_u32);
        let parent_state = base.state.clone();

        let extended = base.extending(11_u64);
        match extended.state.as_ref() {
            EnvironmentState::Overlay { parent, key, entry } => {
                assert!(Rc::ptr_eq(parent, &parent_state));
                assert_eq!(*key, TypeId::of::<u64>());
                match entry {
                    EnvironmentEntry::Present(value) => {
                        assert_eq!(value.downcast_ref::<u64>(), Some(&11_u64));
                    }
                    EnvironmentEntry::Removed => {
                        panic!("overlay entry unexpectedly removed");
                    }
                }
            }
            EnvironmentState::Map(_) => panic!("extending must create overlay state"),
        }
    }

    #[test]
    fn deep_overlay_chain_preserves_parent_visibility() {
        let mut env = Environment::new();
        env.insert(String::from("root"));
        let env = env.extending(3_i32).extending(true).extending(9_u8);

        assert_eq!(env.get::<String>(), Some(&String::from("root")));
        assert_eq!(env.get::<i32>(), Some(&3_i32));
        assert_eq!(env.get::<bool>(), Some(&true));
        assert_eq!(env.get::<u8>(), Some(&9_u8));
    }
}
