# WaterUI 🌊

**A modern, cross-platform UI framework for Rust**

Build beautiful, reactive applications that run everywhere - from desktop to mobile to embedded devices.

## ✨ Features

- 🦀 **100% Rust** - Type-safe, memory-safe, fast
- 🔄 **Reactive by Design** - UI automatically updates when data changes
- 🌍 **Cross-Platform** - One codebase, every platform
- 📱 **Platform Native** - GTK4 on desktop, WebAssembly on web
- 🪶 **Lightweight** - `no-std` support for embedded devices
- ⚡ **Performance** - Zero-cost abstractions, compile-time optimizations

## 🚀 Quick Start

```rust
use waterui::*;
use nami::{binding, s};

// Stateless components use functions
fn counter_app() -> impl View {
    let count = binding(0);

    vstack((
        text!("Count: {}", count),
        button("Increment", {
            let count = count.clone();
            move |_| count.update(|c| c + 1)
        }),
    ))
    .padding(20.0)
}

fn main() {
    App::new().run(counter_app());
}
```

## 🏗️ Architecture

WaterUI is built on three core concepts:

### 1. Views

Declarative UI components that describe what should be displayed:

```rust
// Function-based views for stateless components
fn user_card(name: &str, role: &str) -> impl View {
    vstack((
        text!(name).weight(.bold),
        text!(role).color(Color::secondary()),
    ))
    .padding(15.0)
    .background(Color::card())
    .corner_radius(8.0)
}
```

### 2. Reactive State

Powered by the [Nami](https://github.com/water-rs/nami) reactive system:

```rust
let username = binding("Alice".to_string());
let greeting = s!("Hello, {}!", username);

// Use text! macro for reactive text
let ui = text!(greeting);

// Updates automatically propagate
username.set("Bob".to_string());
```

### 3. Environment System

Type-safe dependency injection through the view hierarchy:

```rust
fn themed_button(label: &str) -> impl View {
    button(label)
        .background(env!(AppTheme).primary_color)
        .color(env!(AppTheme).text_color)
}
```

## 🧱 Component Ecosystem

### Layout Components

- `vstack()`, `hstack()`, `zstack()` - Stack layouts
- `grid()` - Flexible grid layout
- `scroll()` - Scrollable containers
- `padding()`, `margin()` - Spacing modifiers

### Form Controls

- `button()` - Interactive buttons
- `text_field()` - Text input
- `toggle()` - Boolean switches
- `slider()` - Numeric input
- `picker()` - Selection controls

### Media & Graphics

- `image()` - Static and dynamic images
- `video()` - Video playback
- `canvas()` - Custom drawing
- `shape()` - Geometric shapes

## 🎨 Styling System

```rust
fn styled_card() -> impl View {
    vstack((
        text!("Card Title").size(18.0),
        text!("Card content goes here..."),
    ))
    .padding(20.0)
    .background(Color::white())
    .corner_radius(12.0)
    .shadow(2.0)
    .border(1.0, Color::gray(0.2))
}
```

## 📱 Platform Support

| Platform        | Backend     | Status          |
| --------------- | ----------- | --------------- |
| Linux Desktop   | GTK4        | ✅ Stable       |
| macOS Desktop   | GTK4        | ✅ Stable       |
| Windows Desktop | GTK4        | ✅ Stable       |
| Web Browser     | WebAssembly | 🚧 Beta         |
| iOS             | Native      | 🗓️ Planned      |
| Android         | Native      | 🗓️ Planned      |
| Embedded        | `no-std`    | ✅ Experimental |

## 🔄 Reactive Programming

WaterUI embraces reactive programming with the `s!` macro for computations:

```rust
let width = binding(100.0);
let height = binding(50.0);

// Reactive computations
let area = s!(width * height);
let aspect_ratio = s!(width / height);

// Conditional reactivity
let status = s!(if area > 1000.0 { "Large" } else { "Small" });

// Use in UI
vstack((
    text!("Area: {:.1}", area),
    text!("Ratio: {:.2}", aspect_ratio),
    text!("Status: {}", status),
))
```

## 📖 Getting Started

### Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui = { version = "0.1", features = ["gtk4"] }
nami = "0.1"  # Reactive state management
```

### Your First App

```rust
use waterui::*;
use nami::{binding, s};

fn todo_app() -> impl View {
    let todos = binding(vec![
        "Learn WaterUI".to_string(),
        "Build amazing apps".to_string(),
    ]);
    let input = binding(String::new());

    vstack((
        // Header
        text!("Todo App").size(24.0).weight(.bold),

        // Input section
        hstack((
            text_field("Add todo...", input.clone()),
            button("Add", {
                let todos = todos.clone();
                let input = input.clone();
                move |_| {
                    if !input.get().trim().is_empty() {
                        todos.update(|list| list.push(input.get()));
                        input.set(String::new());
                    }
                }
            }),
        )),

        // Todo list
        scroll(vstack(
            s!(todos.iter().enumerate().map(|(i, todo)| {
                hstack((
                    text!(todo),
                    button("×", {
                        let todos = todos.clone();
                        move |_| todos.update(|list| { list.remove(i); })
                    }),
                ))
            }).collect::<Vec<_>>())
        )),
    ))
    .padding(20.0)
}

fn main() {
    App::new()
        .title("Todo App")
        .size(400, 600)
        .run(todo_app());
}
```

## 🔧 Development Setup

### Prerequisites

- Rust 1.85+ (2024 edition)
- Platform dependencies:
  - **Linux**: `sudo apt install libgtk-4-dev build-essential`
  - **macOS**: `brew install gtk4`
  - **Windows**: Install GTK4 via MSYS2

### Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run examples
cargo run --example counter
cargo run --example todo-app
```

### Testing

```bash
# Run all tests
cargo test --all-features

# Test with memory safety checks
cargo +nightly miri test

# Linting
cargo clippy --all-targets --all-features
```

## 📚 Documentation

- 📖 [Tutorial Book](tutorial-book/) - Complete learning guide
- 📋 [API Reference](https://docs.rs/waterui) - Detailed API documentation
- 🏗️ [Architecture Guide](CODEBASE_DOCUMENTATION.md) - Framework internals
- ✨ [Examples](examples/) - Sample applications

## 🤝 Contributing

We welcome contributions! Please see our [Contributing Guide](CONTRIBUTING.md) for details.

### Development Workflow

1. Fork and clone the repository
2. Create a feature branch: `git checkout -b feature-name`
3. Make your changes following our coding standards
4. Run tests: `cargo test --all-features`
5. Submit a pull request

## 🛠️ Current Status

WaterUI is in **active development**. The core framework is functional but APIs may change.

### Completed ✅

- Core reactive system with Nami integration
- GTK4 desktop backend
- Component library (layout, forms, media)
- Environment system for dependency injection
- Comprehensive documentation and tutorial

### In Progress 🚧

- WebAssembly backend improvements
- Performance optimizations
- Additional platform backends

### Planned 🗓️

- iOS and Android native backends
- Hot reloading for development
- Visual designer/builder tool
- Plugin ecosystem

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- Built with ❤️ in Rust
- Inspired by SwiftUI, React, and Flutter
- Powered by GTK4 for desktop rendering
- State management by [Nami](https://github.com/water-rs/nami)

---

**Ready to build something amazing?** 🚀

```bash
cargo new my-waterui-app
cd my-waterui-app
# Add WaterUI to Cargo.toml and start coding!
```
