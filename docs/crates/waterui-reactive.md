# WaterUI Reactive

**Version**: 0.1.0  
**Location**: `reactive/`

## Overview

`waterui-reactive` is a powerful, lightweight reactive framework that forms the foundation of WaterUI's reactive architecture. It provides fine-grained reactivity with automatic dependency tracking and efficient updates.

## Core Architecture

### The Compute Trait

The foundation of the reactive system is the `Compute` trait:

```rust
pub trait Compute: Clone + 'static {
    type Output;

    // Get the current value
    fn compute(&self) -> Self::Output;

    // Register a watcher to be notified of changes
    fn add_watcher(&self, watcher: impl Watcher<Self::Output>) -> WatcherGuard;
}
```

This trait describes a reactive value that can be computed and observed. It provides a consistent interface for working with reactive values regardless of their specific implementation.

### Binding - Two-Way Reactive Containers

`Binding<T>` is a two-way binding container that serves as the source of truth for application state:

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

#### Specialized Methods by Type

Bindings provide type-specific methods for common operations:

- `Binding<bool>`: `toggle()` for boolean values
- `Binding<i32>`: `increment()`, `decrement()` for integers  
- `Binding<Str>`: `append()`, `clear()` for strings
- `Binding<Vec<T>>`: `push()`, `clear()` for vectors

### Computed Values

`Computed<T>` is a type-erased container that can hold any implementation of the `Compute` trait, providing a uniform way to work with different kinds of computations.

### Watchers - Reactive Observers

Watchers allow you to react to changes in reactive values:

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

**Important**: Always store the returned `WatcherGuard`. This guard ensures the watcher is properly unregistered when dropped, preventing memory leaks.

#### Watcher Metadata

Watchers can receive metadata using `Watcher::new` to construct a standard watcher. This is essential for the reactive animation system.

## Key Modules

### Binding (`binding.rs`)
- Two-way reactive data binding
- Type-specific operations
- Automatic change notification

### Compute (`compute.rs`) 
- Core computation trait and implementations
- Type-erased computed values
- Dependency tracking

### Cache (`cache.rs`)
- Caching utilities for expensive computations
- Memoization support
- Cache invalidation strategies

### Channel (`channel.rs`)
- Reactive communication channels
- Cross-component messaging
- Event propagation

### Stream (`stream.rs`)
- Reactive data streams
- Stream transformations
- Async integration

### Map (`map.rs`)
- Value transformation utilities
- Functional mapping operations
- Chain transformations

### Project (`project.rs`)
- Value projection utilities
- Selective reactive updates
- Derived computations

### Zip (`zip.rs`)
- Combining multiple reactive values
- Synchronized updates
- Tuple operations

### Mailbox (`mailbox.rs`)
- **Thread-safe communication**: The reactive framework is designed to be single-threaded by default
- Cross-thread reactive bridges
- Safe concurrent access

## Macros

### `impl_constant!`
Implements the `Compute` trait for constant types:

```rust
impl_constant!(String, i32, f64);
```

This generates `Compute` implementations for types that should be treated as constant values in reactive computations.

### `impl_genetic_constant!`
Similar to `impl_constant!` but works with generic type parameters:

```rust
impl_genetic_constant!(Vec<T>, Option<T>);
```

## Features

- **Fine-grained reactivity**: Only affected components update when data changes
- **Automatic dependency tracking**: No manual subscription management
- **Memory safe**: Automatic cleanup with `WatcherGuard`
- **Type safe**: Compile-time guarantees for reactive chains
- **Performance optimized**: Minimal overhead with lazy evaluation
- **Single-threaded by design**: Use `Mailbox` for cross-thread operations

## Integration

The reactive framework integrates seamlessly with the WaterUI view system:

```rust
use waterui_reactive::{Binding, binding};
use waterui::components::Dynamic;

// Create a reactive state container
let counter = binding(0);

// Create a view that responds to state changes
let view = Dynamic::watch(counter, |count| {
    text(format!("Current value: {}", count))
});
```

The UI automatically updates when state changes, with efficient rendering that only updates affected components.

## Best Practices

1. **Store WatcherGuards**: Always keep the `WatcherGuard` returned by watchers to prevent memory leaks
2. **Use appropriate binding types**: Leverage type-specific methods for cleaner code
3. **Minimize computational cost**: Use caching for expensive derived computations
4. **Thread safety**: Use `Mailbox` for cross-thread reactive operations
5. **Compose computations**: Build complex reactive logic from simple primitives
