# WaterUI Core

The foundational crate of the WaterUI framework, providing essential building blocks for developing cross-platform reactive user interfaces.

## Overview

`waterui-core` establishes a unified architecture that works consistently across desktop, mobile, web, and embedded environments. It provides the core abstractions and systems that all other WaterUI components build upon.

## Key Features

- **Declarative View System**: Compose complex UIs from simple, reusable components
- **Environment-based Context**: Type-safe dependency injection and configuration propagation
- **Type Erasure**: `AnyView` enables heterogeneous collections of different view types
- **Plugin Architecture**: Extensible system for adding framework capabilities
- **Animation System**: Declarative animations with spring physics and transitions
- **Cross-platform**: Consistent API across all supported platforms
- **No-std Compatible**: Can be used in embedded environments

## Core Concepts

### View Trait

The foundation of all UI components:

```rust
pub trait View: 'static {
    fn body(self, env: &Environment) -> impl View;
}
```

### Environment

Type-based dependency injection system:

```rust
let env = Environment::new()
    .with(Theme::Dark)
    .install(LocalizationPlugin::new("en_US"));
```

### AnyView

Type-erased view container for dynamic composition:

```rust
let views: Vec<AnyView> = vec![
    AnyView::new(Text::new("Hello")),
    AnyView::new(Button::new("Click me")),
];
```

## Example

```rust
use waterui_core::{View, Environment, AnyView};
use waterui_text::text;

struct MyView {
    title: String,
}

impl View for MyView {
    fn body(self, _env: &Environment) -> impl View {
        text(self.title)
    }
}
```

## Dependencies

- `waterui-str`: Efficient string handling
- `nami`: Reactive state management

This crate is typically used through the main `waterui` crate, which re-exports the most commonly used types and traits.
