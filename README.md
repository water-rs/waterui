# WaterUI

[![Crates.io](https://img.shields.io/crates/v/waterui.svg)](https://crates.io/crates/waterui)
[![docs.rs](https://docs.rs/waterui/badge.svg)](https://docs.rs/waterui)
[![License](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Coverage](https://img.shields.io/codecov/c/github/water-rs/waterui?logo=codecov)](https://app.codecov.io/gh/water-rs/waterui)

A modern, cross-platform UI framework for Rust that renders to native platform widgets (UIKit/AppKit on Apple, Android View on Android) rather than drawing its own pixels. Build reactive, declarative UIs with Rust's type safety and performance.

## Overview

WaterUI combines declarative, component-based architecture with fine-grained reactivity to deliver truly native user interfaces across platforms. Unlike traditional Rust UI frameworks that implement custom rendering, WaterUI translates your Rust view tree into platform-native UI components through an FFI bridge, ensuring your apps look and feel native on every platform.

The framework is built on three core principles:

- **Native-first rendering**: Your UI components compile to UIKit/AppKit views on iOS/macOS and Android View on Android, delivering authentic native behavior and performance.
- **Fine-grained reactivity**: Powered by the `nami` crate, UI updates are surgical and automatic—only affected components re-render when state changes.
- **Hot reload**: Changes to your Rust code reload instantly in running apps via dynamic library swapping, providing a development experience similar to web frameworks.

WaterUI is ideal for building production mobile apps, cross-platform tools, and native desktop applications where performance and platform integration matter.

## Installation

Add WaterUI to your `Cargo.toml`:

```toml
[dependencies]
waterui = "0.2"
waterui-ffi = "0.2"  # Required for FFI export
```

Enable the graphics feature for GPU rendering capabilities:

```toml
[dependencies]
waterui = { version = "0.2", features = ["graphics"] }
```

## Quick Start

The fastest way to experience WaterUI is through the CLI's playground mode, which handles native backend setup automatically:

### 1. Install the CLI

```bash
cargo install waterui-cli
```

### 2. Create and Run a Playground

```bash
water create --playground --name my-app
cd my-app
water run
```

Your app launches with hot reload enabled. Edit `src/lib.rs` and watch changes appear instantly.

### 3. Write Your First View

```rust,ignore
use waterui::prelude::*;
use waterui::app::App;

#[hot_reload]
fn main() -> impl View {
    vstack((
        text("Hello, WaterUI!").size(24.0).bold(),
        text("Build native UIs with Rust"),
    ))
    .spacing(12.0)
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
```

## Core Concepts

### The View Trait

Every UI component implements the `View` trait, which defines how it renders:

```rust
use waterui_core::Environment;

pub trait View: 'static {
    fn body(self, env: &Environment) -> impl View;
}
```

Views compose recursively—complex interfaces are built from simple building blocks. The framework handles type erasure, rendering, and updates automatically.

### Reactive State

WaterUI uses `Binding<T>` for mutable state and `Computed<T>` for derived values. Views automatically update when reactive values change:

```rust
use waterui::prelude::*;

#[hot_reload]
fn counter() -> impl View {
    let count = Binding::int(0);

    vstack((
        waterui::text!("Count: {}", count),
        button("Increment")
            .action({
                let count = count.clone();
                move || count.set(count.get() + 1)
            }),
    ))
}
```

### Environment

The `Environment` provides dependency injection for themes, fonts, and custom services. Values propagate down the view tree without explicit passing:

```rust
use waterui::app::App;
use waterui::prelude::*;
use waterui_core::Environment;
use waterui_text::font::{ResolvedFont, FontWeight};

pub fn app(mut env: Environment) -> App {
    let theme = Theme::new()
        .color_scheme(ColorScheme::Dark)
        .fonts(FontSettings::new().body(ResolvedFont::new(16.0, FontWeight::Normal)));

    env.install(theme);
    App::new(main, env)
}
```

### View Modifiers

WaterUI provides a fluent API for styling and layout through the `ViewExt` trait:

```rust
use waterui::prelude::*;
use waterui_color::Color;

text("Styled Text")
    .size(18.0)
    .bold()
    .foreground(Color::srgb(100, 150, 255))
    .padding()
    .background(Color::srgb_hex("#f0f0f0"))
    .on_tap(|| println!("Tapped!"));
```

## Examples

### Form with Reactive Bindings

```rust,ignore
use waterui::prelude::*;

#[form]
struct Settings {
    username: String,
    dark_mode: bool,
    volume: f64,
}

#[hot_reload]
fn settings_view() -> impl View {
    let settings = Settings::binding();

    vstack((
        text("Settings").size(24.0),
        form(&settings),
        Divider,
        waterui::text!("Dark mode: {}", settings.project().dark_mode),
        waterui::text!("Volume: {:.0}%", settings.project().volume.map(|v| v * 100.0)),
    ))
    .padding()
}
```

### Interactive Gesture Handling

```rust
use waterui::prelude::*;
use waterui::gesture::{TapGesture, DragGesture, LongPressGesture};

#[hot_reload]
fn gestures() -> impl View {
    let tap_count = Binding::int(0);

    vstack((
        waterui::text!("Taps: {}", tap_count),
        text("Tap Me")
            .padding()
            .background(Color::srgb_hex("#2196F3").with_opacity(0.3))
            .gesture(TapGesture::new(), {
                let tap_count = tap_count.clone();
                move || tap_count.set(tap_count.get() + 1)
            }),
        text("Long Press")
            .padding()
            .background(Color::srgb_hex("#FF9800").with_opacity(0.3))
            .gesture(LongPressGesture::new(500), || {
                println!("Long press detected");
            }),
    ))
}
```

### Dynamic List Rendering

```rust
use waterui::prelude::*;
use waterui::component::list::{List, ListItem};

#[derive(Clone)]
struct Contact {
    id: u64,
    name: &'static str,
    role: &'static str,
}

impl Identifiable for Contact {
    type Id = u64;
    fn id(&self) -> Self::Id {
        self.id
    }
}

fn contacts_list() -> impl View {
    let contacts = vec![
        Contact { id: 1, name: "Alice Chen", role: "Software Engineer" },
        Contact { id: 2, name: "Bob Smith", role: "Product Manager" },
        Contact { id: 3, name: "Carol Williams", role: "Designer" },
    ];

    List::for_each(contacts, |contact| ListItem {
        content: AnyView::new(
            vstack((
                text(contact.name).size(17.0).bold(),
                text(contact.role)
                    .size(14.0)
                    .foreground(Color::srgb(128, 128, 128)),
            ))
            .padding_with(EdgeInsets::symmetric(12.0, 16.0)),
        ),
        on_delete: None,
    })
}
```

## API Overview

### Layout Components

- `vstack()`, `hstack()`, `zstack()` - Vertical, horizontal, and depth stacks with configurable spacing and alignment
- `scroll()` - Scrollable container with automatic content overflow handling
- `spacer()` - Flexible space filler for pushing elements apart
- `padding()` - Add insets around views with `EdgeInsets` configuration
- `Frame` - Fixed, minimum, and maximum sizing constraints

### Controls

- `Button` - Tappable button with action handler and style variants
- `TextField` - Single-line text input with label and placeholder
- `Toggle` - Boolean switch control with reactive binding
- `Slider` - Continuous value selector within a range
- `Stepper` - Discrete value adjuster with step increment
- `Picker` - Selection control for choosing from multiple options

### Text & Media

- `Text` - Styled text with font, size, weight, and color configuration
- `text!()` macro - Reactive text with format string interpolation
- `styled()` - Rich text with multiple style runs
- `VideoPlayer` - Native video playback with controls and event handling
- `include_markdown!()` - Compile-time markdown to view conversion

### Form Components

- `#[form]` derive macro - Automatic form generation from structs
- `TextField`, `Toggle`, `Slider` - Form-compatible controls with labels
- Automatic field-to-control mapping based on type

### Navigation

- `NavigationView` - Hierarchical navigation with title bar
- `TabView` - Tab-based navigation container
- `.title()` modifier - Set navigation bar title

### Advanced

- `Dynamic::watch()` - Observe reactive signals and rebuild views on change
- `AnyView` - Type-erased view container for heterogeneous collections
- `Environment` - Dependency injection and context propagation
- `ViewExt` - Extension trait providing modifier methods for all views

## Features

### Default Features

The base `waterui` crate includes layout, controls, text, media, navigation, and form components with native rendering backends.

### Optional Features

- `graphics` - Enables GPU rendering with canvas drawing primitives and `GpuSurface` (requires `waterui-graphics`)
- `graphics-minimal` - GPU surface only, without canvas (smaller binary size)

## Application Entry Point

Every WaterUI app follows this pattern:

```rust,ignore
use waterui::prelude::*;
use waterui::app::App;

// Initialize environment (called once at app startup)
pub fn init() -> Environment {
    Environment::new()
}

// Root view (called on every render)
pub fn main() -> impl View {
    text("Your app content here")
}

// Alternative: App with custom environment setup
pub fn app(mut env: Environment) -> App {
    // Install plugins, themes, etc.
    env.install(Theme::new().color_scheme(ColorScheme::Dark));
    App::new(main, env)
}

// Export FFI entry points for native backends
waterui_ffi::export!();
```

The `waterui_ffi::export!()` macro generates C-compatible functions that native backends (Swift/Kotlin) call to render your UI.

### Core Architecture

- [`waterui-core`](core/) - Foundation types: `View` trait, `Environment`, `AnyView`, reactivity primitives
- [`waterui-ffi`](ffi/) - C FFI bridge with `export!()` macro for native backend integration

### Component Libraries

- [`waterui-layout`](components/layout/) - Layout containers and geometry
- [`waterui-controls`](components/controls/) - Buttons, toggles, sliders, text fields
- [`waterui-text`](components/text/) - Text rendering, fonts, and styling
- [`waterui-form`](components/form/) - Form builder with `#[form]` derive macro
- [`waterui-media`](components/media/) - Video/audio playback components
- [`waterui-navigation`](components/navigation/) - Navigation containers and routing
- [`waterui-graphics`](components/graphics/) - Canvas drawing and GPU rendering (optional)

### Tools

- [`waterui-cli`](cli/) - Command-line tool for creating, building, and running apps

## CLI Commands

The `water` CLI provides a complete development workflow:

```bash
# Create new project
water create --name "My App" --backend apple --backend android

# Create playground (auto-configured backends)
water create --playground --name my-playground

# Run with hot reload
water run --platform ios --device "iPhone 15 Pro"
water run --platform android

# Build Rust library for specific target
water build ios
water build android

# Check development environment
water doctor

# List available devices
water devices
```

## Platform Support

- **iOS/macOS**: Renders to UIKit/AppKit (requires Xcode)
- **Android**: Renders to Android View (requires Android SDK)

## Documentation

- [API Reference](https://docs.rs/waterui) - Complete API documentation
- [Architecture Guide](CODEBASE_DOCUMENTATION.md) - Technical architecture overview
- [Roadmap](ROADMAP.md) - Planned features and improvements

## Contributing

Contributions are welcome! Please submit pull requests to the `dev` branch. The `main` branch is reserved for releases.

## License

License under Apache 2.0 OR MIT license.