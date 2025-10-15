# `WaterUI` 🌊

[![Crates.io](https://img.shields.io/crates/v/waterui.svg)](https://crates.io/crates/waterui)
[![docs.rs](https://docs.rs/waterui/badge.svg)](https://docs.rs/waterui)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A modern, cross-platform UI framework for Rust that delivers true native rendering with reactive state management.

## 🚀 Quick Start

Add `WaterUI` to your `Cargo.toml`:

```toml
[dependencies]
waterui = "0.1.0"
```

Create your first reactive counter:

```rust
use waterui::prelude::*;
use waterui_core::binding;
use waterui_layout::stack::{hstack, vstack};
use waterui::component::button;
use waterui::Binding;
use waterui_core::SignalExt;

pub fn counter() -> impl View {
    let count: Binding<i32> = binding(0);
    let doubled = count.clone().map(|value| value * 2);

    let increment_button = {
        let count = count.clone();
        button("Increment").action(move || count.set(count.get() + 1))
    };

    let reset_button = {
        let count = count.clone();
        button("Reset").action(move || count.set(0))
    };

    vstack((
        text!("Count: {count}"),
        text!("Doubled: {doubled}"),
        hstack((increment_button, reset_button)),
    ))
}
```

## 📱 Android CLI Workflow

`WaterUI` ships a CLI that can scaffold and package Android applications without
opening Android Studio. To try it end-to-end:

1. **Install the Android command-line tools** (example for Linux):

   ```bash
   mkdir -p "$HOME/android-sdk" && cd "$HOME/android-sdk"
   curl -O https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip
   unzip commandlinetools-linux-11076708_latest.zip
   mkdir -p cmdline-tools/latest
   mv cmdline-tools/* cmdline-tools/latest/
   ```

2. **Install the required SDK components**:

   ```bash
   cmdline-tools/latest/bin/sdkmanager \
     --sdk_root="$HOME/android-sdk" \
     "platform-tools" "platforms;android-34" "build-tools;34.0.0"
   yes | cmdline-tools/latest/bin/sdkmanager --sdk_root="$HOME/android-sdk" --licenses
   ```

3. **Expose the SDK to the CLI**:

   ```bash
   export ANDROID_SDK_ROOT="$HOME/android-sdk"
   export ANDROID_HOME="$HOME/android-sdk"
   ```

4. **Create and package a project**:

   ```bash
   cargo run -p waterui-cli -- create \
     --name "Android Demo" \
     --directory android-demo \
     --bundle-identifier com.example.androiddemo \
     --backend android \
     --yes --dev

   TERM=dumb cargo run -p waterui-cli -- package \
     --platform android \
     --project android-demo \
     --skip-native
   ```

Gradle will output a ready-to-install APK at
`android-demo/android/app/build/outputs/apk/debug/app-debug.apk`.

## 🌐 Web Backend CLI Workflow

The CLI can also scaffold an end-to-end web experience that compiles to
WebAssembly and serves assets with a lightweight development server.

1. **Install the tooling dependencies**:

   ```bash
   cargo install wasm-pack
   rustup target add wasm32-unknown-unknown
   ```

2. **Create a web-first project**:

   ```bash
   cargo run -p waterui-cli -- create \\
     --name "Web Demo" \\
     --directory web-demo \\
     --bundle-identifier com.example.webdemo \\
     --backend web \\
     --yes --dev
   ```

3. **Serve it like a Vite dev server**:

   ```bash
   cargo run -p waterui-cli -- run \\
     --platform web \\
     --project web-demo
   ```

   The command compiles the Wasm bundle with `wasm-pack`, launches a local HTTP
   server, and watches your Rust + web assets for rebuilds.

4. **Capture previews without committing binaries**:

   Screenshots or other artifacts from the web preview should be placed in an
   `artifacts/` directory (ignored by git) so that large binary assets stay out
   of the repository.

## 📝 Rich Text & Markdown

`WaterUI` includes native support for styled text and Markdown rendering. Use
`StyledStr::from_markdown` for inline formatting, or render full documents with
`RichText::from_markdown`:

```rust
use waterui::widget::RichText;
use waterui::View;

pub fn release_notes() -> impl View {
    RichText::from_markdown(
        r"# What's new

- **Rich text** rendering
- Inline _emphasis_
- Tables and images
",)
}
```

## ✨ Features

- **🎯 True Native Rendering** - Uses `SwiftUI` on Apple platforms (macOS, iOS, visionOS, watchOS, widgets!)
- **⚡ Fine-Grained Reactivity** - Vue-like reactive updates without virtual DOM overhead
- **🔒 Type Safety** - Leverage Rust's powerful type system from UI to data
- **🔄 Declarative & Reactive** - Familiar API for `SwiftUI` and React developers
- **🌐 Cross-Platform** - Multiple backends: `SwiftUI`, GTK4, Web, and more planned
- **🚫 No-std Support** - Deploy to embedded environments
- **🎨 Composable Architecture** - Build complex UIs from simple, reusable components

## 📦 Architecture

`WaterUI` follows a modular architecture with clear separation of concerns:

- **Core Framework** (`waterui-core`) - View trait, Environment system, reactive state
- **Component Libraries** - Text, Layout, Forms, Media, Navigation components
- **Platform Backends** - `SwiftUI`, GTK4, Web renderers
- **Utilities** - String handling, color management, cross-platform tools

## 🛣️ Roadmap

**Current Version: 0.1.0** - First glance ✅

- ✅ Basic widgets: stack, text, scroll, form
- ✅ `SwiftUI` backend
- ✅ MVP of GTK4 backend
- ✅ Stabilized core design

**Next: 0.2.0** - Usable

- 🔧 Memory leak fixes
- 🔧 Stabilized layout system
- 🔧 Android backend MVP
- 🔧 CLI tooling
- 🔧 Gesture support
- 🔧 Hot reload
- 🔧 Internationalization (i18n)
- 🔧 Styling system

**Future Milestones:**

- **0.3.0** - Media widgets, Canvas API, Platform-specific APIs
- **0.4.0** - Self-rendering backend MVP
- **0.5.0** - Rich text and markdown support
- **0.6.0+** - Enhanced self-rendering, developer tools, animations

[View full roadmap →](./ROADMAP.md)

## 🎮 Examples & Demos

**`SwiftUI` Backend Demo**\
Native macOS/iOS applications → [View Demo](./demo)

**GTK4 Backend Examples**\
Cross-platform desktop apps → [View Examples](./backends/gtk4/examples/)

## 📚 Documentation

- **[Tutorial Book](https://water-rs.github.io/waterui/)** - Learn `WaterUI` step by step
- **[API Reference (Latest)](https://water-rs.github.io/waterui/api)** - Development docs
- **[API Reference (docs.rs)](https://docs.rs/waterui)** - Stable release docs

## 🤝 Contributing

We welcome contributions! `WaterUI` is in active development and there's plenty to work on:

1. **Fork the repository**
2. **Create a feature branch**: `git checkout -b feature/amazing-feature`
3. **Make your changes** and add tests
4. **Run the linter**: `cargo clippy --all-targets --all-features --workspace -- -D warnings`
5. **Submit a pull request**

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
