# WaterUI Core

**Version**: 0.1.0  
**Location**: `core/`

## Overview

`waterui-core` is the foundational crate of the WaterUI framework, providing essential building blocks for developing cross-platform reactive user interfaces. It establishes a unified architecture that works consistently across desktop, mobile, web, and embedded environments.

## Key Components

### View System

The core of WaterUI is built around the `View` trait, which forms the foundation of the UI component model:

```rust
pub trait View: 'static {
    fn body(self, env: &Environment) -> impl View;
}
```

This recursive definition enables composition of complex interfaces from simple building blocks. Each view receives contextual information through the `Environment` and transforms into its visual representation.

### Environment & Context Propagation

The `Environment` provides a type-based dependency injection system that propagates configuration and resources through the view hierarchy:

```rust
let env = Environment::new()
    .with(Theme::Dark)
    .install(LocalizationPlugin::new("en_US"));
```

This eliminates the need for explicit parameter passing and ensures consistent configuration throughout the application.

### Type Erasure with AnyView

`AnyView` enables heterogeneous collections by preserving behavior while erasing concrete types, facilitating dynamic composition patterns. This is essential for creating collections of different view types.

### Component Architecture

The framework provides several component categories:

- **Platform Components**: Native UI elements with platform-optimized rendering
- **Reactive Components**: Views that automatically update when data changes  
- **Metadata Components**: Elements that carry additional rendering instructions
- **Composite Components**: Higher-order components built from primitive elements

### Animation System

Provides declarative animation capabilities with:
- Transition animations between view states
- Interactive gesture-driven animations
- Timeline-based animations
- Spring physics animations

### Color Management

Built-in color handling with support for:
- Standard color spaces (sRGB, P3, etc.)
- Dynamic color adaptation
- Theme-aware color resolution
- Accessibility contrast adjustments

### Shape System

Geometric shape primitives for custom drawing:
- Rectangles, circles, and paths
- Stroke and fill styling
- Complex shape combinations
- Path animations

### Plugin Architecture

The extensible plugin interface enables framework extensions without modifying core code:

```rust
pub trait Plugin: Sized + 'static {
    fn install(self, env: &mut Environment);
    fn uninstall(self, env: &mut Environment);
}
```

This enables modular functionality like theming, localization, and platform-specific features.

## Features

- **No-std compatibility**: Can be used in embedded environments
- **Type-safe**: Leverages Rust's type system for compile-time guarantees
- **Performance**: Minimal overhead with zero-cost abstractions
- **Cross-platform**: Consistent API across all supported platforms
- **Reactive**: Built-in reactive data flow with efficient updates

## Dependencies

- `waterui-reactive`: Reactive programming primitives
- `waterui-str`: Efficient string handling
- `waterui-task`: Async task management
- `waterui-color`: Color utilities
- `anyhow`: Error handling
- `paste`: Macro utilities

## Integration

This crate is typically used indirectly through the main `waterui` crate, which re-exports the most commonly used types and traits. Direct usage is recommended for:

- Building custom UI frameworks on top of WaterUI
- Creating platform-specific renderers
- Developing WaterUI plugins and extensions

## Example

```rust
use waterui_core::{View, Environment, AnyView};

struct MyView {
    title: String,
}

impl View for MyView {
    fn body(self, env: &Environment) -> impl View {
        text(self.title)
    }
}

// Usage
let view = MyView { title: "Hello World".to_string() };
let any_view = AnyView::new(view);
```
