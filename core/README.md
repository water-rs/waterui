# waterui-core

The foundational crate providing essential building blocks for the WaterUI cross-platform reactive UI framework.

## Overview

`waterui-core` establishes the architectural foundation for WaterUI applications, enabling declarative, reactive user interfaces that render to native platform widgets. This crate provides the core abstractions used throughout the framework: the `View` trait for composable UI components, the `Environment` for type-safe context propagation, reactive primitives powered by the `nami` library, and type erasure utilities for dynamic composition.

Unlike traditional immediate-mode or retained-mode frameworks, WaterUI uses a reactive composition model where views automatically update when reactive state changes, and the entire view tree is transformed into native platform widgets (UIKit/AppKit on Apple platforms, Jetpack Compose on Android) rather than custom rendering.

This crate is `no_std` compatible (with allocation) and works consistently across desktop, mobile, web, and embedded environments.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-core = "0.1.0"
```

For most applications, use the main `waterui` crate which re-exports all core functionality along with component libraries.

## Quick Start

```rust
use waterui_core::{View, Environment, binding, Binding};

// Define a custom view
fn counter(count: Binding<i32>) -> impl View {
    Dynamic::watch(count, |value| {
        format!("Count: {}", value)
    })
}

// Create an application environment
fn init() -> Environment {
    Environment::new()
}

// Define the root view
fn main() -> impl View {
    let count = binding(0);
    counter(count)
}
```

## Core Concepts

### The View Trait

The `View` trait is the foundation of all UI components in WaterUI. It defines a single method that transforms the view into its rendered representation:

```rust
pub trait View: 'static {
    fn body(self, env: &Environment) -> impl View;
}
```

Views compose recursively - a view's `body` method returns another view, allowing complex UIs to be built from simple primitives. The framework handles the recursion, eventually reaching "raw views" (like `Text`, `Button`) that map directly to native widgets.

Implementing `View` for custom types:

```rust
use waterui_core::{View, Environment};

struct Greeting {
    name: String,
}

impl View for Greeting {
    fn body(self, _env: &Environment) -> impl View {
        format!("Hello, {}!", self.name)
    }
}
```

Many standard types implement `View` automatically:
- `&'static str`, `String`, `Cow<'static, str>` - render as text
- `()` - empty view
- `Option<V>` - renders `Some(view)` or nothing
- `Result<V, E>` - renders either the success or error view
- Closures `Fn() -> impl View` - lazy view construction

### Environment

The `Environment` is a type-indexed store that propagates context through the view hierarchy without explicit parameter passing:

```rust
use waterui_core::Environment;

#[derive(Clone)]
struct AppConfig {
    api_url: String,
}

let env = Environment::new()
    .with(AppConfig {
        api_url: "https://api.example.com".to_string(),
    });

// Later, in a view:
use waterui_core::env::use_env;
use waterui_core::extract::Use;

let config_view = use_env(|Use(config): Use<AppConfig>| {
    format!("API: {}", config.api_url)
});
```

The environment supports:
- **Typed storage**: Insert values of any `'static` type
- **Plugin installation**: Modular extensions via the `Plugin` trait
- **View hooks**: Intercept and modify view configurations globally
- **Cloning**: Cheap cloning via `Rc` for child environments

### AnyView - Type Erasure

`AnyView` enables storing different view types in homogeneous collections:

```rust
use waterui_core::AnyView;

let views: Vec<AnyView> = vec![
    AnyView::new("Hello"),
    AnyView::new(42.to_string()),
    AnyView::new(()),
];
```

Type erasure is essential for dynamic UIs where the concrete view type isn't known at compile time. `AnyView` automatically unwraps nested erasure to avoid performance overhead.

### Reactive Primitives

WaterUI integrates the `nami` reactive system for fine-grained updates:

```rust
use waterui_core::{binding, Binding, Dynamic};

// Create reactive state
let count: Binding<i32> = binding(0);

// Create a view that watches the state
let counter_view = Dynamic::watch(count.clone(), |value| {
    format!("Count: {}", value)
});

// Updating the binding automatically updates the view
count.set(5);
```

Key reactive types (re-exported from `nami`):
- `Binding<T>` - Mutable reactive state
- `Computed<T>` - Derived reactive values
- `Signal<T>` - Read-only reactive values
- `SignalExt` - Extension methods for all reactive types

The `Dynamic` component bridges reactive state to the view system. When a watched value changes, the view automatically re-renders with the new data.

### Native Views

Native views are leaf components that map directly to platform widgets. The `NativeView` trait marks types that should be handled by the platform backend:

```rust
use waterui_core::{NativeView, layout::StretchAxis};

struct CustomWidget;

impl NativeView for CustomWidget {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}
```

The `raw_view!` macro simplifies creating native views:

```rust
raw_view!(Spacer, StretchAxis::MainAxis);
raw_view!(Divider, StretchAxis::CrossAxis);
```

## Examples

### Custom Component with State

```rust
use waterui_core::{View, Environment, binding, Binding, Dynamic};

struct Toggle {
    label: String,
    is_on: Binding<bool>,
}

impl Toggle {
    fn new(label: impl Into<String>) -> (Binding<bool>, Self) {
        let is_on = binding(false);
        (is_on.clone(), Self {
            label: label.into(),
            is_on,
        })
    }
}

impl View for Toggle {
    fn body(self, _env: &Environment) -> impl View {
        Dynamic::watch(self.is_on, move |value| {
            format!("{}: {}", self.label, if value { "ON" } else { "OFF" })
        })
    }
}
```

### Environment-based Configuration

```rust
use waterui_core::{View, Environment, env::use_env, extract::Use};

#[derive(Clone, Debug)]
struct Theme {
    primary_color: String,
}

fn themed_view() -> impl View {
    use_env(|Use(theme): Use<Theme>| {
        format!("Using theme color: {}", theme.primary_color)
    })
}

fn init() -> Environment {
    Environment::new().with(Theme {
        primary_color: "#007AFF".to_string(),
    })
}
```

### Reactive Computed Values

```rust
use waterui_core::{binding, Binding, Computed, Dynamic, SignalExt};

let count = binding(0);
let doubled: Computed<i32> = count.map(|n| n * 2);

let view = Dynamic::watch(doubled, |value| {
    format!("Doubled: {}", value)
});
```

### Plugin System

```rust
use waterui_core::{plugin::Plugin, Environment};

struct AnalyticsPlugin {
    app_id: String,
}

impl Plugin for AnalyticsPlugin {
    fn install(self, env: &mut Environment) {
        env.insert(self);
    }
}

let mut env = Environment::new();
AnalyticsPlugin {
    app_id: "my-app".to_string(),
}.install(&mut env);
```

## API Overview

### Core Traits
- `View` - The fundamental UI component trait
- `IntoView` - Convert types into views
- `TupleViews` - Convert tuples of views into collections
- `ConfigurableView` - Views with configuration objects
- `ViewConfiguration` - Configuration types for configurable views

### Type Erasure
- `AnyView` - Type-erased view container
- `Native<T>` - Wrapper for platform-native components
- `NativeView` - Trait for native platform widgets

### Reactive Components
- `Dynamic` - Runtime-updatable view component
- `DynamicHandler` - Handle for updating dynamic views
- `watch()` - Helper to create reactive views

### Environment & Context
- `Environment` - Type-indexed dependency injection store
- `UseEnv` - View that accesses environment values
- `use_env()` - Helper to create environment-aware views
- `With<V, T>` - Wrap a view with additional environment value

### Metadata & Hooks
- `Metadata<T>` - Attach metadata to views (must be handled by renderer)
- `IgnorableMetadata<T>` - Optional metadata (can be ignored by renderer)
- `Retain` - Keep values alive for view lifetime
- `Hook<C>` - Intercept and modify view configurations

### Layout Primitives
- `Rect`, `Size`, `Point` - Geometry types (logical pixels)
- `ProposalSize` - Size proposals for layout negotiation
- `StretchAxis` - Specify which axis a view expands on
- `Layout` - Trait for custom layout algorithms
- `SubView` - Proxy for querying child view sizes

### Event Handling
- `Event` - Enumeration of UI events (`Appear`, `Disappear`)
- `OnEvent` - Event handler component
- `Handler<T>`, `HandlerOnce<T>` - Handler traits for environment-based callbacks
- `ActionObject` - Type alias for action handlers

### View Collections
- `Views` - Trait for collections with stable identities
- `AnyViews<V>` - Type-erased view collection
- `ForEach<C, F, V>` - Transform data collections into views
- `ViewsExt` - Extension methods for view collections

### Animation
- `Animation` - Declarative animation specifications
- `AnimationExt` - Extension trait for reactive values
- `.animated()` - Apply default animation
- `.with_animation()` - Apply specific animation

## Features

### Default Features
- **None** - The crate has no default features for maximum flexibility

### Optional Features
- `std` - Enable standard library support (currently no-op, reserved for future use)
- `nightly` - Enable nightly-only features (e.g., `!` never type)
- `serde` - Enable Serde serialization support for core types

## Architecture Notes

### Layout System

WaterUI uses **logical pixels** (points) for all layout values, matching design tools like Figma:
- 1 logical pixel = 1 point in design tools
- Backends handle conversion to physical pixels based on screen density
- Consistent physical size across platforms and densities

Layout is a two-phase process:
1. **Sizing**: Determine container size given a proposal from parent
2. **Placement**: Position children within the final bounds

The `Layout` trait defines custom layout algorithms. The `StretchAxis` enum specifies which axes views expand on:
- `None` - Content-sized
- `Horizontal` / `Vertical` - Expand on one axis
- `Both` - Greedy, fills all space
- `MainAxis` / `CrossAxis` - Relative to container direction

### View Rendering Pipeline

```
Custom View → body() → ... → body() → Raw View → Native Backend → Platform Widget
```

1. Custom views define `body()` that returns other views
2. Framework recursively calls `body()` until reaching raw views
3. Raw views (marked with `NativeView`) are handled by the platform backend
4. Backend translates to native widgets (SwiftUI views, Compose composables, etc.)

### Handler System

The handler system supports automatic parameter extraction from environments:

```rust
use waterui_core::{handler::into_handler, extract::Use};

struct Config { value: i32 }

let handler = into_handler(|Use(config): Use<Config>| {
    println!("Config value: {}", config.value);
});
```

Handlers come in three flavors:
- `Handler<T>` - Reusable, takes `&mut self`
- `HandlerOnce<T>` - Single-use, consumes `self`
- `HandlerFn<P, T>` - Function-like trait with parameter extraction

## License

MIT
