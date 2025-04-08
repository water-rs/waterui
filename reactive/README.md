# WaterUI Reactive Framework

A powerful, lightweight reactive state management system.

## Overview

The WaterUI reactive framework provides a comprehensive set of primitives for managing application state in a reactive way. It enables automatic propagation of changes through your application, with minimal boilerplate and maximum flexibility.

```
+----------------+      +----------------+      +---------------+
|    Binding     |----->|    Computed    |----->|    Watcher    |
|   (Mutable)    |      |   (Derived)    |      |  (Callbacks)  |
+----------------+      +----------------+      +---------------+
        ^                       |                       |
        |                       |                       |
        +------------------+----+-----------------------+
                           |
                        Updates
```

## Core of our architecture: `Compute` trait

```rust
pub trait Compute: Clone + 'static {
    type Output: ComputeResult;

    // Get the current value
    fn compute(&self) -> Self::Output;

    // Register a watcher to be notified of changes
    fn add_watcher(&self, watcher: Watcher<Self::Output>) -> WatcherGuard;
}
```

This trait is implemented by `Binding`, `Computed`, and all other reactive types, providing a consistent interface for working with reactive values regardless of their specific implementation.

## Core Components

### Binding

`Binding<T>` is the foundation of the reactive system - a mutable, observable value:

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

### Computed

`Computed<T>` represents a value derived from other reactive values:

```rust
use waterui_reactive::{ComputeExt, binding};

let price = binding(10.0);
let quantity = binding(2);

// Create a computed value that updates when dependencies change
let total = price.clone().zip(quantity.clone()).map(|(p, q)| p * q as f64);

assert_eq!(total.compute(), 20.0);

// When price changes, total is automatically updated
price.set(15.0);
assert_eq!(total.compute(), 30.0);
```

`Computed<T>` is a type-erased container that can hold any implementation of the `Compute` trait, providing a uniform way to work with different kinds of computations.

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

When working with watchers, it's important to store the returned `WatcherGuard`. This guard ensures the watcher is properly unregistered when dropped, preventing memory leaks.

## Thread Safety Model

The reactive framework is designed to be **single-threaded** by default. For cross-thread operations, use the [`Mailbox`](mailbox/index.html) type which provides a safe bridge.

## Additional Features

The framework provides many more capabilities:

- [**Composition and Transformation**](map/index.html): Combine and transform reactive values
- [**Collections**](collection/index.html): Efficiently manage collections of reactive data
- [**Constants**](constant/index.html): Create optimized immutable reactive values
- [**Extension Methods**](trait.ComputeExt.html): Convenient methods for all reactive types
