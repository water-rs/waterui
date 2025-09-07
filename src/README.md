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

```rust,ignore
use nami::binding;
use waterui::{View, ViewExt};
use waterui::component::button;
use waterui_layout::stack::{vstack, hstack};
use waterui_text::text;

fn create_counter_view() -> impl View {
    let counter = binding(0);

    vstack((
        // Display updates automatically when counter changes
        text!("{}", counter),

        hstack((
            // Button that increments counter when clicked
            button("Increment").action({
                let counter = counter.clone();
                move |_| counter.update(|v| v + 1)
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

```rust,ignore
use nami::binding;
use waterui::{ViewExt};
use waterui::component::button;
use waterui_layout::stack::vstack;
use waterui_form::{toggle, slider};
use waterui_text::text;

let is_enabled = binding(true);
let volume = binding(0.7);

let settings_panel = vstack((
    text("Settings"),

    toggle(text("Enable Notifications"), &is_enabled),

    slider(0.0..=1.0, &volume).label(text("Volume")),

    button("Apply")
        .action({
            let is_enabled = is_enabled.clone();
            move |_| if is_enabled.get() { /* apply */ }
        })
))
.padding();
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
