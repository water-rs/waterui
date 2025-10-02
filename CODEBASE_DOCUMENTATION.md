# WaterUI Framework - Comprehensive Codebase Documentation

## Table of Contents

1. [Overview](#overview)
2. [Architecture](#architecture)
3. [Core System](#core-system)
4. [Component Libraries](#component-libraries)
5. [Backend Implementations](#backend-implementations)
6. [Utility Libraries](#utility-libraries)
7. [Build System & Dependencies](#build-system--dependencies)
8. [Development Workflow](#development-workflow)
9. [Code Quality Standards](#code-quality-standards)

---

## Overview

WaterUI is a modern, experimental UI framework written in Rust that enables cross-platform application development using a single codebase. The framework is designed to run on desktop, mobile, web, and embedded environments, with special support for `no-std` environments.

### Key Characteristics

- **Declarative**: UI components are described declaratively using the View trait
- **Reactive**: State changes automatically propagate through the UI hierarchy
- **Type-safe**: Leverages Rust's type system for compile-time correctness
- **Cross-platform**: Single codebase deploys to multiple platforms
- **Memory-efficient**: Optimized for both resource-rich and constrained environments

### Project Structure

```
waterui/
├── core/                    # Core framework functionality
├── components/              # UI component libraries
│   ├── canvas/             # Canvas and drawing components
│   ├── form/               # Form controls and inputs
│   ├── layout/             # Layout containers and utilities
│   ├── media/              # Media components (image, video)
│   ├── navigation/         # Navigation and routing
│   └── text/               # Text rendering and typography
├── backends/               # Platform-specific implementations
│   ├── gtk4/              # GTK4 desktop backend
│   └── web/               # Web/WASM backend
├── utils/                 # Utility libraries
│   └── str/               # Optimized string utilities
├── plugins/               # Optional plugins
│   ├── icon/              # Icon support
│   └── i18n/              # Internationalization
├── kit/                   # Platform-specific utilities
├── window/                # Window management
├── render_utils/          # Shared rendering utilities
```

---

## Architecture

### Core Design Principles

The WaterUI framework is built around three fundamental architectural concepts:

#### 1. View System (`waterui-core::view`)

The View trait is the foundation of the component model:

```rust
pub trait View: 'static {
    fn body(self, env: &Environment) -> impl View;
}
```

This recursive definition enables:

- **Composability**: Complex interfaces built from simple components
- **Type Safety**: Compile-time verification of component hierarchy
- **Performance**: Zero-cost abstractions with monomorphization

#### 2. Environment System (`waterui-core::env`)

Type-based dependency injection system that propagates context:

```rust
#[derive(Debug, Clone, Default)]
pub struct Environment {
    map: BTreeMap<TypeId, Rc<dyn Any>>,
}
```

Features:

- Type-safe context passing
- Plugin system integration
- Configuration inheritance
- Modifier system support

#### 3. Reactive State (`nami` integration)

Seamless integration with the Nami reactive state management library:

```rust
// Reactive binding
let counter = binding(0);

// Automatically updating UI
text(counter.display())
```

### Component Categories

1. **Platform Components**: Native UI elements optimized for each platform
2. **Reactive Components**: Views that automatically update with state changes
3. **Metadata Components**: Elements carrying rendering instructions
4. **Composite Components**: Higher-order components built from primitives

---

## Core System

### `waterui-core` (Core Framework)

The core crate provides essential building blocks:

#### Key Modules:

- **`view`**: Core View trait and component system
- **`env`**: Environment and context management
- **`components`**: Basic component implementations including AnyView
- **`color`**: Color management and representations
- **`shape`**: Geometric primitives and shapes
- **`animation`**: Animation system foundations
- **`handler`**: Event handling abstractions
- **`plugin`**: Plugin system interface

#### Critical Types:

**View Trait**

```rust
pub trait View: 'static {
    fn body(self, env: &Environment) -> impl View;
}
```

**AnyView (Type Erasure)**

```rust
pub struct AnyView {
    // Internal representation for heterogeneous view collections
}
```

**Environment (Context System)**

```rust
impl Environment {
    pub fn with<T: 'static>(mut self, value: T) -> Self;
    pub fn get<T: 'static>(&self) -> Option<&T>;
    pub fn install(mut self, plugin: impl Plugin) -> Self;
}
```

**ConfigurableView & Modifier System**

```rust
pub trait ConfigurableView: View {
    type Config: 'static;
    fn config(self) -> Self::Config;
}

pub struct Modifier<V: ConfigurableView>(
    Box<dyn Fn(Environment, V::Config) -> AnyView>
);
```

#### Features:

- **No-std Support**: Can operate without standard library
- **Type Safety**: Leverages Rust's type system extensively
- **Zero-cost Abstractions**: Performance-optimized trait implementations
- **Plugin Architecture**: Extensible through plugins

---

## Component Libraries

### `waterui-layout` (Layout System)

Provides comprehensive layout capabilities:

#### Core Types:

```rust
pub struct Frame {
    pub width: f64,
    pub height: f64,
    pub min_width: f64,
    pub max_width: f64,
    // ... sizing constraints
    pub margin: Edge,
    pub alignment: Alignment,
}

pub struct Edge {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

pub enum Alignment {
    Default,
    Leading,
    Center,
    Trailing,
}
```

#### Components:

- **Stack Layouts**: VStack, HStack, ZStack for linear and layered arrangements
- **Grid System**: Two-dimensional layout containers
- **Scroll Views**: Scrollable content containers
- **Spacers**: Flexible spacing components
- **Overlays**: Layered content system

### `waterui-text` (Typography System)

Comprehensive text rendering and formatting:

#### Key Features:

```rust
configurable!(Text, TextConfig);

pub struct TextConfig {
    pub content: Computed<Str>,
    pub font: Computed<Font>,
}

impl Text {
    pub fn new(content: impl IntoComputed<Str>) -> Self;
    pub fn display<T: Display>(source: impl IntoComputed<T>) -> Self;
    pub fn format<T>(value: impl IntoComputed<T>, formatter: impl Formatter<T>) -> Self;
    pub fn font(mut self, font: impl Signal<Output = Font>) -> Self;
    pub fn size(mut self, size: impl IntoComputed<f64>) -> Self;
}
```

#### Modules:

- **`font`**: Font utilities and definitions
- **`attributed`**: Rich text formatting support
- **`link`**: Interactive text components
- **`locale`**: Localization and formatting utilities
- **`macros`**: Convenient text creation macros

### `waterui-form` (Form Controls)

Interactive form elements and controls:

#### Components:

- **TextField**: Text input controls
- **Toggle**: Boolean toggle switches
- **Slider**: Range input controls
- **Stepper**: Numeric increment/decrement controls
- **Pickers**: Date, color, and multi-selection pickers
- **SecureField**: Password input controls

#### Features:

- Reactive data binding
- Validation support
- Custom formatting
- Platform-optimized rendering

### `waterui-media` (Media Components)

Media rendering and display:

#### Components:

- **Image**: Static image display
- **Video**: Video playback controls
- **LivePhoto**: Live photo support (iOS)
- **AsyncImage**: Async image loading

### `waterui-navigation` (Navigation System)

Navigation and routing components:

#### Components:

- **NavigationView**: Navigation container
- **TabView**: Tab-based navigation
- **Link**: Navigation links
- **Route handling**: URL-based routing

### `waterui-canvas` (Canvas System)

Drawing and graphics primitives:

#### Features:

- Custom drawing operations
- Path-based graphics
- Vector graphics support
- Animation integration

---

## Backend Implementations

### `waterui-gtk4` (GTK4 Desktop Backend)

Native desktop implementation using GTK4:

#### Architecture:

```rust
pub mod app;      // Application lifecycle
pub mod events;   // Event handling
pub mod layout;   // Layout management
pub mod renderer; // Widget rendering
pub mod widgets;  // GTK4 widget implementations
```

#### Key Components:

- **Gtk4App**: Application entry point and lifecycle management
- **Renderer**: Converts WaterUI components to GTK4 widgets
- **Event System**: Maps GTK4 events to WaterUI handlers
- **Widget Implementations**: Platform-specific widget rendering

#### Supported Platforms:

- Linux (primary)
- Windows (via GTK4 for Windows)
- macOS (via GTK4 for macOS)

#### Dependencies:

- `gtk4`: GTK4 Rust bindings
- `gdk4`: Graphics toolkit
- `pango`: Text rendering
- `vello`: 2D graphics rendering
- `time`: Time utilities

### `waterui-web` (Web/WASM Backend)

Web deployment via WebAssembly:

#### Features:

- WebAssembly compilation target
- DOM manipulation
- Browser API integration
- Event handling via web-sys

#### Dependencies:

- `wasm-bindgen`: Rust-WASM bindings
- `web-sys`: Web API bindings
- `js-sys`: JavaScript API bindings
- `console_error_panic_hook`: Error handling
- `serde`/`serde_json`: Data serialization

---

## Utility Libraries

### `waterui-str` (Optimized String Library)

Memory-efficient string type supporting both static and owned strings:

#### Core Design:

```rust
#[repr(C)]
pub struct Str {
    ptr: NonNull<()>,
    len: isize,  // Positive for static, negative for shared
}
```

#### Features:

- **Dual Mode**: Static references and reference-counted owned strings
- **Zero-cost Cloning**: For static strings
- **Reference Counting**: For owned strings with atomic operations
- **Memory Safety**: Comprehensive Miri testing for memory correctness
- **No-std Support**: Works in embedded environments

#### Key Operations:

```rust
impl Str {
    pub const fn from_static(s: &'static str) -> Self;
    pub fn from_utf8(bytes: Vec<u8>) -> Result<Self, FromUtf8Error>;
    pub fn as_str(&self) -> &str;
    // intentionally not exposed: reference counts are private
    pub fn into_string(self) -> String;
    pub fn append(&mut self, s: impl AsRef<str>);
}
```

#### Memory Safety:

- Extensive test suite with 970+ lines of memory safety tests
- Miri validation for undefined behavior detection
- Reference counting correctness verification
- Thread safety considerations

---

## Build System & Dependencies

### Cargo Workspace Configuration

The project uses a Cargo workspace with the following structure:

```toml
[workspace]
members = [
    "core",
    "kit",
    "components/*",
    "utils/str",
    "backends/*",
    "render_utils",
    "window"
]
resolver = "2"
```

#### Workspace-wide Settings:

- **Edition**: 2024 (requires Rust 1.85+)
- **License**: MIT
- **Rust Version**: Minimum 1.85

### Core Dependencies

#### Framework Dependencies:

- **`nami`**: Reactive state management (local dependency)
- **`native-executor`**: Async task execution
- **`typeshare`**: Cross-language type sharing
- **`paste`**: Macro utilities
- **`anyhow`**: Error handling
- **`smol`**: Async runtime components

#### Optional Dependencies:

- **`serde`**: Serialization support (feature-gated)

### Linting Configuration

Comprehensive linting setup enforcing code quality:

```toml
[workspace.lints]
rust.missing_docs = "warn"
rust.missing_debug_implementations = "warn"
clippy.all = "warn"
clippy.style = "warn"
clippy.correctness = "warn"
clippy.complexity = "warn"
clippy.suspicious = "warn"
clippy.perf = "warn"
clippy.pedantic = "warn"
clippy.nursery = "warn"
clippy.cargo = "warn"
```

---

## Development Workflow

### Build Commands

#### Standard Operations:

```bash
# Build all crates with features
cargo build --all-features --workspace

# Run all tests
cargo test --all-features --workspace

# Linting
cargo clippy --all-targets --all-features --workspace -- -D warnings

# Code formatting
cargo fmt --all -- --check

# Documentation generation
cargo doc --all-features --no-deps --workspace
```

#### Memory Safety Testing:

```bash
# Run Miri memory safety checks
cargo +nightly miri test -p waterui-str

# Uses configuration in utils/str/miri.toml
```

### Testing Strategy

#### Unit Testing:

- Comprehensive unit tests for all major components
- Memory safety tests (970+ lines in waterui-str)
- Property-based testing where applicable

#### Integration Testing:

- Cross-component integration tests
- Backend-specific testing
- Example-based testing

#### Memory Safety:

- Miri testing for undefined behavior detection
- Reference counting correctness verification
- Memory leak detection

### Documentation Standards

#### Code Documentation:

- All public APIs must have documentation
- Examples in documentation where applicable
- Architecture documentation in module headers
- README files for major components

#### API Documentation:

- Comprehensive rustdoc comments
- Usage examples
- Safety requirements for unsafe code
- Performance characteristics

---

## Code Quality Standards

### Code Style Guidelines

#### Rust Edition and Version:

- **Edition**: 2024
- **MSRV**: 1.85 (minimum supported Rust version)
- **Features**: Leverages latest stable Rust features

#### Naming Conventions:

- **Types**: PascalCase (e.g., `TextView`, `Environment`)
- **Functions**: snake_case (e.g., `into_view`, `get_value`)
- **Constants**: SCREAMING_SNAKE_CASE
- **Modules**: snake_case

#### Code Organization:

- Logical module separation
- Clear public API boundaries
- Consistent error handling patterns
- Comprehensive trait implementations

### Performance Considerations

#### Memory Management:

- Reference counting for shared data (waterui-str)
- Zero-cost abstractions where possible
- Minimal allocations in hot paths
- Copy-on-write semantics where beneficial

#### Compilation Performance:

- Incremental compilation support
- Optimized debug builds
- Efficient generic instantiation
- Minimal dependency trees

#### Runtime Performance:

- Platform-optimized rendering
- Efficient event handling
- Lazy evaluation where possible
- Memory-conscious data structures

### Safety and Security

#### Memory Safety:

- Extensive unsafe code documentation
- Miri testing for undefined behavior
- Reference counting correctness
- No data races in concurrent contexts

#### Type Safety:

- Comprehensive type system usage
- Trait bounds for correctness
- Compile-time verification
- Minimal runtime type checking

#### API Safety:

- Clear error handling
- Panic documentation
- Safe defaults
- Defensive programming practices

### Testing Requirements

#### Coverage Requirements:

- Unit tests for all public APIs
- Integration tests for major features
- Memory safety tests for unsafe code
- Performance regression tests

#### Quality Gates:

- All tests must pass
- Linting must pass with zero warnings
- Documentation must be complete
- Memory safety must be verified

### Contribution Guidelines

#### Code Review Process:

- All changes require code review
- Memory safety review for unsafe code
- Performance impact assessment
- Documentation completeness check

#### Testing Requirements:

- New code must include tests
- Memory safety tests for unsafe code
- Integration tests for new features
- Benchmark tests for performance-critical code

---

## Future Roadmap

### Planned Features

- Better error handling without std
- Hot reloading support
- CLI tools for project management
- Multi-window support
- Enhanced async support
- Additional backend implementations

### Architecture Evolution

- Plugin system expansion
- Enhanced type safety
- Performance optimizations
- API stabilization
