# `WaterUI` Reactive Framework

A powerful, lightweight reactive framework for `WaterUI`.

## Core of our architecture: `Compute` trait

```rust
pub trait Compute: Clone + 'static {
    type Output;

    // Get the current value
    fn compute(&self) -> Self::Output;

    // Register a watcher to be notified of changes
    fn add_watcher(&self, watcher: impl Watcher<Self::Output>) -> WatcherGuard;
}
```

`Compute` describes a reactive value that can be computed and observed. It can generate a new value by `compute()` method.
This value implements `ComputeResult` trait, so it can be cloned cheaply and compare easily.
It will notify watchers only when its value actually changes.


This trait is implemented by `Binding`, `Computed`, and all other reactive types, providing a consistent interface for working with reactive values regardless of their specific implementation.

`Computed<T>` is a type-erased container that can hold any implementation of the `Compute` trait, providing a uniform way to work with different kinds of computations.

### Binding

`Binding<T>` is a two-way binding container.

```rust
use waterui_reactive::binding;

// Create a binding with an initial value
let counter = binding(0);

// Modify the binding
counter.set(5);
counter.increment(1); // Now equals 6

// Read the current value
assert_eq!(counter.get(), 6);
```

Bindings serve as the source of truth for application state and notify observers when their values change. They provide specialized methods for different data types:

- `Binding<bool>` - `toggle()` for boolean values
- `Binding<i32>` - `increment()`, `decrement()` for integers
- `Binding<Str>` - `append()`, `clear()` for strings
- `Binding<Vec<T>>` - `push()`, `clear()` for vectors

### Watchers

Watchers let you react to changes in reactive values:

```rust
use waterui_reactive::{binding, ComputeExt};

let name = binding("World".to_string());

// Watch for changes and execute a callback
let _guard = name.watch(|value| {
    println!("Hello, {}!", value);
});

// This will trigger the watcher
name.set("Universe".to_string());
```

What's more, watchers can receive a metadata by using `Watcher::new` to construct a standard watcher.It is essential for our reactive animation system.

When working with watchers, it's important to store the returned `WatcherGuard`. This guard ensures the watcher is properly unregistered when dropped, preventing memory leaks.

## Mailbox

The reactive framework is designed to be **single-threaded** by default. For cross-thread operations, use the [`Mailbox`](mailbox/index.html) type which provides a safe bridge.
