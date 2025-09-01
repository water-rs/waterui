# WaterUI Framework

WaterUI is a high-performance, reactive UI framework for creating cross-platform applications
with a single Rust codebase.

Built from the ground up for efficiency and expressiveness, WaterUI enables developers to craft
beautiful, responsive interfaces that work consistently across desktop, mobile, web, and even
embedded platforms with minimal platform-specific code.

## Core Principles

- **Declarative Syntax**: Describe what your UI should look like, not how to build it
- **Reactive State Management**: UI automatically updates when data changes with minimal overhead
- **True Native Performance**: Compiles directly to platform-native code without intermediate layers
- **Type-Safe Interfaces**: Catch UI errors at compile time rather than runtime
- **No-Std Compatible**: Run on resource-constrained devices with minimal overhead

## Simple Counter Example

```rust
use waterui::{
    binding, button, text, View, ViewExt,
    layout::{vstack, hstack},
};

fn create_counter_view() -> impl View {
    // Create reactive state
    let counter = binding(0);

    vstack((
        // Display that updates automatically when counter changes
        text(counter.display()),

        hstack((
            // Button that increments counter when clicked
            button("Increment").action({
                let counter = counter.clone();
                move |_| counter.add(1)
            }),

            // Button that resets counter when clicked
            button("Reset").action({
                let counter = counter.clone();
                move |_| counter.set(0)
            })
        ))
    ))
    .padding()
}
```

## Framework Architecture

WaterUI consists of several integrated layers:

- **Core**: Fundamental view protocol and environment handling ([`waterui_core`])
- **Reactive**: State management and change propagation ([`nami`])
- **Components**: Ready-to-use UI elements ([`component`] module)
- **Layout**: Flexible positioning and arrangement system ([`layout`] module)
- **Animation**: Fluid transitions and motion effects
- **Platform Adapters**: Native rendering for each target platform

## Key Concepts

- **[`View`]**: Protocol for any UI element that can be rendered
- **[`Binding`]**: Mutable, observable values that trigger UI updates
- **[`Computed`]**: Read-only values derived from other reactive sources
- **[`Environment`]**: Type-based dependency injection throughout the view hierarchy
- **[`ViewExt`]**: Extension methods for applying common view modifiers

## Component Library

WaterUI provides a comprehensive set of built-in components:

```rust
use waterui::{
    binding, Binding, ViewExt,
    component::{button, text, progress, slider, toggle, Badge},
    layout::vstack,
    background::Background,
    core::Color,
};

// Create stateful toggles and sliders
let is_enabled = binding(true);
let volume = binding(0.7);

// Build a rich UI with minimal code
let settings_panel = vstack((
    text("Settings").size(24),

    toggle("Enable Notifications", &is_enabled),

    slider(0.0..=1.0, &volume)
        .label("Volume")
        .background(Background::color(Color::from((0.9, 0.9, 0.95)))),

    button("Apply")
        .disabled(!is_enabled)
        .foreground(Color::white())
        .background(Background::color(Color::blue()))
))
.padding()
.badge(5) // Shows a notification badge
```

## Asynchronous Operations

WaterUI provides seamless integration with async Rust through the [`task`] module:

- **Task spawning**: Execute work without blocking the UI
- **Suspense mechanism**: Show loading states during async operations
- **Safe UI updates**: Automatic thread coordination for state updates

## Advanced Patterns

The [`widget`] module offers higher-level composition patterns:

- **Conditional rendering**: Show/hide content based on state
- **Error boundaries**: Gracefully handle and display errors
- **Suspense**: Handle asynchronous content loading

## Platform Integration

WaterUI offers C-compatible FFI bindings through the [`ffi`] module for seamless
integration with platform-specific code. This enables embedding WaterUI in existing
applications or using platform capabilities not directly exposed by the framework.
