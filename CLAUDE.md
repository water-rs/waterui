# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
# Install CLI from source
cargo install --path cli

# Build entire workspace
cargo build

# Run tests
cargo test

# Check with hot reload feature (for development)
RUSTFLAGS="--cfg waterui_enable_hot_reload" cargo check

# Generate FFI C header (after modifying ffi/ APIs)
cargo run --bin generate_header --features cbindgen --manifest-path ffi/Cargo.toml

# Build Apple backend
cd backends/apple && swift build

# Build Android runtime
./gradlew -p backends/android runtime:assembleDebug

# Run demo app (after creating a project)
water run --platform ios
water run --platform android
```

## Architecture Overview

WaterUI is a cross-platform reactive UI framework that renders to native platform widgets (SwiftUI on Apple, Jetpack Compose on Android) rather than drawing its own pixels.

### Core Data Flow

```
Rust View Tree → FFI (C ABI) → Native Backend (Swift/Kotlin) → Platform UI
```

### Crate Structure

- **`waterui`** - Main crate, re-exports components and provides `prelude`
- **`waterui-core`** - Foundation: `View` trait, `Environment`, `AnyView` type erasure, reactive primitives (`Binding`, `Computed`)
- **`waterui-ffi`** - C FFI layer bridging Rust to native backends; `export!()` macro generates entry points

### Component Libraries (`components/`)

- `layout` - HStack, VStack, ZStack, ScrollView, Spacer
- `controls` - Button, Toggle, Slider, Stepper, Picker, Progress
- `text` - Text, styled text, fonts, markdown
- `form` - Form builder with `#[form]` derive macro
- `navigation` - Navigation containers, TabView
- `media` - Video/audio playback
- `graphics` - Canvas drawing primitives

### Backends (`backends/`)

- **`apple/`** - Git submodule, SwiftUI backend (Swift Package)
- **`android/`** - Git submodule, Android Views + JNI (Gradle project)
- **`hydrolysis/`** - Self-drawn renderer (Vello/tiny-skia) - experimental
- **`tui/`** - Terminal UI backend - WIP

### CLI (`cli/`)

The `water` CLI orchestrates builds across platforms:
- `water create` - Scaffold new project
- `water run` - Build and deploy to device/simulator with hot reload
- `water build <target>` - Compile Rust library for platform (called by Xcode/Gradle)
- `water doctor` - Check development environment

### FFI Contract

Native backends call into Rust via:
1. `waterui_init()` - Initialize runtime, returns Environment pointer
2. `waterui_env_install_theme()` - Inject native theme colors/fonts
3. `waterui_main()` - Get root view tree
4. Render loop: `waterui_view_id()` to identify view type, then either extract data (`waterui_force_as_*`) for raw views or recurse via `waterui_view_body()` for composite views

Raw views are leaf components (Text, Button, etc.) that map to native widgets. Composite views have a `body()` returning other views.

### Reactive System

Uses `nami` crate for fine-grained reactivity:
- `Binding<T>` - Mutable reactive state
- `Computed<T>` - Derived reactive values
- Views automatically update when reactive values change

### View Trait

```rust
pub trait View: 'static {
    fn body(self, env: &Environment) -> impl View;
}
```

### Application Entry Point Pattern

```rust
pub fn init() -> Environment {
    Environment::new()
}

pub fn main() -> impl View {
    // Return your root view
}

waterui_ffi::export!();  // Generates FFI entry points
```

## Key Development Notes

- Rust edition 2024, minimum rustc 1.87
- Workspace lints enforce strict clippy rules including pedantic/nursery
- `backends/apple` and `backends/android` are git submodules
- The FFI header `ffi/waterui.h` is checked into version control; CI verifies it's up-to-date
- When adding new components, update: Rust view → FFI exports → regenerate header → Swift component → Android component + JNI
