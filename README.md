# `WaterUI` 🌊

[![Crates.io](https://img.shields.io/crates/v/waterui.svg)](https://crates.io/crates/waterui)
[![docs.rs](https://docs.rs/waterui/badge.svg)](https://docs.rs/waterui)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A modern, cross-platform UI framework for Rust, designed for building reactive, performant, and beautiful applications.

`WaterUI` combines the safety and speed of Rust with a declarative, component-based architecture inspired by modern web frameworks. It offers true native rendering on Apple and Android, a powerful self-drawn renderer for desktop, and even a terminal backend, all powered by a fine-grained reactive system.

## 🚀 Quick Start: Playground Mode

The fastest way to try `WaterUI` is with the CLI's **Playground Mode**. This allows you to start coding immediately without setting up complex native backends (like Xcode or Android Studio projects) manually.

### 1. Install the CLI

```bash
cargo install --path cli
```

### 2. Create a Playground

```bash
water create --playground --name my-playground
cd my-playground
```

### 3. Run and Experiment

```bash
water run
```

The playground will automatically set up a temporary native environment for you. You can start editing `src/lib.rs`, and the app will **hot reload** instantly!

### 🎡 Examples to Try

Copy code from our examples into your playground's `src/lib.rs` to see what `WaterUI` can do:

- **[Form Example](examples/form-example/src/lib.rs)**: See how `#[form]` makes building settings screens and input forms effortless.
- **[Video Player](examples/video-player-example/src/lib.rs)**: Try the immersive video player with custom overlay controls.

## 📦 Standard Project Setup

When you're ready to build a full application with permanent native backends:

### 1. Create a Project

Scaffold a new project with specific backends:

```bash
water create --name "My App" --backend apple --backend android
```

### 2. Run

Build, run, and hot-reload on a connected device or simulator:

```bash
water run --platform ios --device "iPhone 15 Pro"
# or
water run --platform android
```

### 3. Add to Cargo.toml

If you are adding `WaterUI` to an existing Rust library:

```toml
[dependencies]
waterui = "0.1.1" # Replace with the latest version
```

## ✨ Features

- **Declarative & Reactive**: Build complex UIs with simple, reusable components. State management is handled by a fine-grained reactivity system.
- **Truly Cross-Platform**:
  - **Native**: Renders to **`SwiftUI`** on Apple platforms and **Jetpack Compose** on Android for a completely native look and feel.
  - **Self-Drawn**: The `Hydrolysis` renderer provides GPU-accelerated rendering on desktop platforms.
- **Powerful CLI**: A dedicated `water` command-line tool to create, run, build, and package your applications.
- **Modern Component Library**: Pre-built components for layouts, controls, forms, text, media, and more.
- **Type-Safe**: Leverage Rust's type system and memory safety guarantees.

## 📖 Documentation

- 📚 [API Documentation](https://docs.rs/waterui/latest) - Complete API reference
- 📖 [Official Book](https://book.waterui.dev) - Guides and tutorials

## 🏗️ Architecture

`WaterUI` uses a modular architecture:

- **`waterui`**: The main crate.
- **`waterui-core`**: The heart of the framework (View trait, Environment, Reactivity).
- **`components/`**: Component libraries (`layout`, `controls`, `form`, `media`, etc.).
- **`backends/`**: Platform renderers (`apple`, `android`, `hydrolysis`, `tui`).
- **`cli/`**: The `water` command-line interface.

## 🛣️ Roadmap

- **Component Library Expansion**: Tables, trees, charts.
- **Renderer Maturation**: `Hydrolysis` performance and platform support.
- **Animation API**: Declarative animation system.
- **Accessibility**: Enhanced accessibility features.

## 🤝 Contributing

We welcome contributions! Please check `ROADMAP.md` and open issues to see where you can help.

For contributions, please merge your changes into the `dev` branch. Releases are made from the `main` branch.

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](./LICENSE) file for details.