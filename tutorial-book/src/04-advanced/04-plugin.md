# Plugin

WaterUI's plugin system is built at the top of [environment system](../02-core-concept/03-environment).

```rust
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
        env.remove::<Self>();
    }
}
```

By intersting environment, plugin can achieve something interesting purpose.

## Implement an i18n Plugin

```rust
[WIP]
```
