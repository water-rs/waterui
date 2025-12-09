# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
# Install CLI from source (required for `water run` to work)
# You must reinstall cli to path after modifiying it if you wanna debug it.
cargo install --path cli

# Build CLI for development (faster iteration, but not in PATH)
cargo build -p waterui-cli

# Build entire workspace
cargo build

# Run tests
cargo test

# Run tests for specific crate
cargo test -p waterui-core
cargo test -p waterui-cli

# Check with hot reload lib feature (for development)
RUSTFLAGS="--cfg waterui_hot_reload_lib" cargo check

# Generate FFI C header (after modifying ffi/ APIs), never write C header by hand
cargo run --bin generate_header --features cbindgen --manifest-path ffi/Cargo.toml

# Build Apple backend
cd backends/apple && swift build

# Build Android runtime
./gradlew -p backends/android runtime:assembleDebug

# Run demo app (after creating a project)
water run --platform ios
water run --platform android

# Create a playground for quick experimentation
water create --playground --name my-playground
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

- `water create` - Scaffold new project (supports `--playground` for quick experiments)
- `water run` - Build and deploy to device/simulator with hot reload. It is an interactive terminal, so LLMs should ask user to run it for you.
- `water build <target>` - Compile Rust library for platform (called by Xcode/Gradle)
- `water package` - Package built artifacts for distribution
- `water clean` - Remove build artifacts
- `water doctor` - Check development environment
- `water devices` - List available devices and simulators

**CLI Architecture Notes:**
- Entry point: `cli/src/terminal/main.rs` - Uses `clap` for parsing, `smol` async runtime
- Commands in `cli/src/terminal/commands/` - Each command is async and returns `Result<()>`
- Hot reload: `cli/src/debug/hot_reload.rs` - WebSocket server with 150ms debounced builds
- Platform abstraction: `Platform` trait in `cli/src/platform.rs` implemented by `ApplePlatform` and `AndroidPlatform`
- Shell output: `cli/src/terminal/shell.rs` - Global singleton with human-readable (ANSI) or JSON modes

Note: `/terminal/*` (waterui-cli binary) only provide a friendly interface for CLI commands. All real logic should be implemented in the waterui-cli library part.

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
- The FFI header `ffi/waterui.h` is checked into version control; CI verifies it's up-to-date; **never write C header by hand**
- When adding new components, update: Rust view → FFI exports → regenerate header → Swift component → Android component + JNI

### Hot Reload System

The hot reload system uses a WebSocket-based architecture:
1. CLI launches `HotReloadServer` on port 2006+ (tries up to 50 variations)
2. Server broadcasts dylib updates to connected apps via WebSocket
3. `BuildManager` debounces file changes (150ms) and manages incremental builds
4. Environment variables `WATERUI_HOT_RELOAD_HOST` and `WATERUI_HOT_RELOAD_PORT` are passed to running apps
5. Apps wrapped in `Hotreload` component check for updates and reload dynamically

### Testing Patterns

- Most tests use `#[cfg(test)] mod tests` pattern
- Run workspace tests: `cargo test`
- Run specific crate tests: `cargo test -p <crate-name>`
- No explicit CLI unit tests found; likely relies on integration testing

### Error Handling

- All command functions return `Result<(), eyre::Report>` for rich error context
- Custom error enums use `thiserror` derive macro
- Shell provides `success!()`, `error!()`, `warn!()`, `note!()` macros for user feedback
