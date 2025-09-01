# WaterUI Framework

## Bring Your App to Every Platform, Once for All

WaterUI is a modern, experimental UI framework written in Rust that enables you to build applications using a single codebase that can run on any platform, including embedded devices.

## Key Features

- **Type-safe, declarative and reactive API** - Build interfaces that automatically react to state changes
- **First-class async support** - Seamlessly integrate with asynchronous operations
- **Platform-independent core** - Create UIs that work consistently across platforms
- **`no-std` support** - Deploy to embedded environments with minimal resources
- **Flexible component system** - Compose and reuse UI elements to build complex interfaces

## Quick Start

```rust
use waterui::{
    text, button, vstack,
    binding, View, Environment,
};

pub struct Counter;

impl View for Counter {
    fn body(self, _env: &Environment) -> impl View {
        let count = binding(0);
        vstack([
            text(count.display()),
            button("Click me!").action({
                let count = count.clone();
                move |_| count.add(1)
            }),
        ])
    }
}
```

## Architecture

WaterUI is built around a few key concepts:

1. **Views**: Composable UI elements that describe what should be displayed
2. **Reactive State**: Values that automatically propagate changes
3. **Environment**: A context passed through the view hierarchy for configuration

The framework uses a platform-agnostic core with platform-specific backends to render your UI on different devices. This means your code looks exactly the same whether it's running on:

- Mobile devices (iOS, Android)
- Desktop platforms (Windows, macOS, Linux)
- Web browsers (via WebAssembly)
- Embedded devices (with no standard library)

## Component System

WaterUI provides a rich set of built-in components:

- **Layout**: `VStack`, `HStack`, `ZStack`, Grid, Scroll
- **Controls**: Button, Toggle, TextField, Slider, Stepper
- **Navigation**: Navigation views, tabs, links
- **Media**: Photo, Video, LivePhoto

All components are designed to be composable, allowing you to build complex interfaces from simple building blocks.

## Reactive Programming Model

The framework uses a reactive programming model where UI components automatically update when their underlying data changes:

```rust
// Create a reactive binding
let name = binding("World");

// This text will automatically update when name changes - USE text! macro for reactivity
let greeting = text!("Hello, {}!", name);

// Later, update the binding
name.set("Universe");  // Text now shows "Hello, Universe!"
```

## Comparison with Other Frameworks

| Feature | WaterUI | React Native | Flutter | SwiftUI |
|---------|---------|--------------|---------|---------|
| Language | Rust | JavaScript | Dart | Swift |
| Runtime | Native | JS VM | Dart VM | Native |
| no-std support | ✅ | ❌ | ❌ | ❌ |
| Declarative UI | ✅ | ✅ | ✅ | ✅ |
| Type Safety | Strong | Weak | Strong | Strong |

## Current Status

WaterUI is under active development and is currently in an experimental state. While it's ready for experimentation, we recommend waiting for a stable release before using it in production applications.

### TODO

- [ ] Better error handling
- [ ] Support async and error handling without std
- [ ] Icon component
- [ ] Hot reloading
- [ ] CLI tools for project management
- [ ] Multi-window support

## Getting Started

1. Add WaterUI to your Cargo.toml:
```toml
[dependencies]
waterui = "0.1.0"
```

2. Create a new UI:
```rust
use waterui::{text, View, Environment};

struct HelloWorld;

impl View for HelloWorld {
    fn body(self, _env: &Environment) -> impl View {
        text("Hello, World!")
    }
}
```

## Contributing

We welcome contributions to WaterUI! Whether you're fixing bugs, improving documentation, or proposing new features, your help is appreciated.