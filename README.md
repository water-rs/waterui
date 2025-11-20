# `WaterUI` 🌊

[![Crates.io](https://img.shields.io/crates/v/waterui.svg)](https://crates.io/crates/waterui)
[![docs.rs](https://docs.rs/waterui/badge.svg)](https://docs.rs/waterui)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A modern, cross-platform UI framework for Rust, designed for building reactive, performant, and beautiful applications.

WaterUI combines the safety and speed of Rust with a declarative, component-based architecture inspired by modern web frameworks. It offers true native rendering on Apple and Android, a powerful self-drawn renderer for desktop, and even a terminal backend, all powered by a fine-grained reactive system.

## 📖 Documentations

Read our latest documentation [https://docs.rs/waterui/latest](here)!

Read our offical book [https://book.waterui.dev](here)!

## ✨ Features

- **Declarative & Reactive**: Build complex UIs with simple, reusable components. State management is handled by a fine-grained reactivity system, ensuring your UI always stays in sync with your data.
- **Truly Cross-Platform**:
    - **Native**: Renders to **SwiftUI** on Apple platforms and **Jetpack Compose** on Android for a completely native look and feel.
    - **Self-Drawn**: The `Hydrolysis` renderer provides GPU-accelerated (Vello/wgpu) and CPU-based (tiny-skia) backends for high-performance, consistent rendering on desktop platforms (Windows, macOS, Linux).
    - **Terminal**: A TUI backend for building fast, responsive terminal applications.
- **Powerful CLI**: A dedicated `water` command-line tool to create, run, build, and package your applications, with integrated hot-reloading.
- **Modern Component Library**: A rich set of pre-built components for layouts, controls, forms, text, and more.
- **Type-Safe & Safe**: Leverage Rust's powerful type system and memory safety guarantees from your UI to your data logic.

## 🚀 Quick Start

Add `waterui` and its dependencies to your `Cargo.toml`:
```toml
[dependencies]
waterui = "0.1.0" # Replace with the latest version
```
> Or use `waterui = { git = "https://github.com/water-rs/waterui", branch = "main" }` for the latest development version.

Create your first reactive counter:

```rust
use waterui::prelude::*;

fn counter_app() -> impl View {
    let count = Binding::new(0);

    vstack((
        text!("Count: {}", count),
        hstack((
            button("Increment").on_tap({
                let count = count.clone();
                move || count.set(count.get() + 1)
            }),
            button("Reset").on_tap(move || count.set(0)),
        ))
        .spacing(10.0),
    ))
    .padding_with(16.0)
}
```
This example creates a simple view with a counter that can be incremented or reset. The `text!` macro automatically updates the displayed count whenever the `count` binding changes.

## 📦 Architecture

WaterUI is built with a modular architecture to ensure clear separation of concerns and maximum flexibility.

- **`waterui`**: The main crate, which provides the prelude and re-exports key components.
- **`waterui-core`**: The heart of the framework, containing the `View` trait, the reactive state system (powered by `nami`), and the environment system.
- **`components/`**: A collection of component libraries, including:
    - `waterui-layout`: Stacks (`HStack`, `VStack`, `ZStack`), grids, and other layout primitives.
    - `waterui-controls`: Interactive components like `Button`, `Slider`, `TextField`, and `Toggle`.
    - `waterui-text`: Styled text, fonts, and Markdown rendering.
    - `waterui-form`: Form building utilities, including a derive macro for easy form creation.
    - `waterui-graphics`: 2D drawing canvas and shape primitives.
- **`backends/`**: Platform-specific renderers.
    - **`apple`**: SwiftUI backend for macOS, iOS, visionOS, etc.
    - **`android`**: Jetpack Compose backend.
    - **`hydrolysis`**: A self-drawn renderer with GPU (Vello) and CPU (tiny-skia) implementations (Very early stage...DO NOT use it...).
    - **`tui`**: Terminal UI backend (WIP).
- **`cli/`**: The `water` command-line interface for managing your projects.
- **`ffi/`**: A C-compatible Foreign Function Interface that bridges the Rust core with native backends (Swift/Kotlin).

## CLI Workflow

WaterUI ships with a powerful CLI (`water`) to streamline your development process.

### Create a Project
Scaffold a new project with your chosen backends:
```bash
water create --name "My App" --backend apple --backend android
```

### Run with Hot Reload
Build, run, and hot-reload your app on a connected device, simulator, or emulator:
```bash
# The CLI will prompt you to select a target
water run

# Or specify a target directly
water run --platform ios --device "iPhone 15 Pro"
```
The CLI watches for file changes and automatically rebuilds and reloads your application.

### Other Commands
- `water build`: Build native libraries for a specific platform.
- `water package`: Package your application for distribution.
- `water devices`: List available devices, simulators, and emulators.
- `water doctor`: Check your development environment for any issues.
- `water clean`: Clean all build artifacts.

For more details, run `water --help` or see the [CLI README](./cli/README.md).

## 🛣️ Roadmap

- **Component Library Expansion**: Add more advanced components like tables, trees, and charts.
- **Renderer Maturation**: Continue to develop `Hydrolysis` with a focus on performance and broader platform support (including Windows).
- **Animation API**: Introduce a declarative animation and transition system.
- **Accessibility**: Enhance accessibility features across all backends.
- **Documentation**: Improve tutorials, guides, and API documentation.

## 🤝 Contributing

We welcome contributions! `WaterUI` is in active development and there's plenty to work on. Please check the `ROADMAP.md` and open issues to see where you can help.

1.  **Fork the repository**
2.  **Create a feature branch**: `git checkout -b feature/amazing-feature`
3.  **Make your changes** and add tests
4.  **Run the linter**: `cargo clippy --all-targets --all-features --workspace -- -D warnings`
5.  **Submit a pull request**

### Development Commands
```bash
# Build all crates
cargo build --all-features --workspace

# Run tests
cargo test --all-features --workspace

# Check code quality
cargo clippy --all-targets --all-features --workspace -- -D warnings
cargo fmt --all -- --check

# Generate docs
cargo doc --all-features --no-deps --workspace
```

## 🏗️ Project Status

**⚠️ Early Development** - `WaterUI` is in active early development. APIs may change as we stabilize the framework. We're working towards production-ready releases with comprehensive platform support.

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](./LICENSE) file for details.