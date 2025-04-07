//! Provides the `Plugin` trait for extending application functionality.
//!
//! This module defines the core plugin system that allows for modular, extensible
//! application architectures. Plugins can be dynamically installed into and removed
//! from an `Environment`, enabling a flexible component-based design.
//!
//! The plugin system supports:
//! - Dynamic installation and removal of components
//! - Separation of concerns through modular design
//! - Extension of application functionality without modifying core code
//!
//! # Usage
//!
//! Plugins are typically implemented as standalone structs that implement the `Plugin` trait.
//! Once implemented, they can be installed into an `Environment` to extend its capabilities.

use crate::Environment;

/// The `Plugin` trait defines the interface for components that can be installed into
/// and removed from an `Environment`.
///
/// # Examples
///
/// ```
/// struct MyPlugin;
///
/// impl Plugin for MyPlugin {
///     // Plugins don't require any implementation-specific methods by default,
///     // but you can override the `install` and `uninstall` methods if your plugin
///     // needs custom installation or removal behavior.
///     //
///     // For example, a plugin might:
///     // - Register event handlers
///     // - Initialize resources
///     // - Set up configurations
///     // - Connect to external services
///     //
///     // The default implementation simply stores/removes the plugin
///     // instance in the environment.
/// }
///
/// let mut env = Environment::new();
/// MyPlugin.install(&mut env);
/// ```
pub trait Plugin: Sized + 'static {
    /// Installs this plugin into the provided environment.
    ///
    /// This method adds the plugin instance to the environment's storage,
    /// making it available for later retrieval.
    ///
    /// # Arguments
    ///
    /// * `env` - A mutable reference to the environment
    fn install(self, env: &mut Environment) {
        env.insert(self);
    }

    /// Removes this plugin from the provided environment.
    ///
    /// # Arguments
    ///
    /// * `env` - A mutable reference to the environment
    fn uninstall(self, env: &mut Environment) {
        env.remove::<Self>()
    }
}
