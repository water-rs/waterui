# waterui-ffi

FFI bindings layer that bridges Rust view trees to native platform backends.

## Overview

`waterui-ffi` is the C FFI layer at the heart of WaterUI's cross-platform architecture. It provides a type-safe, efficient bridge between Rust application logic and native UI backends, enabling WaterUI apps to render as true native widgets rather than custom-drawn pixels.

This crate serves three critical roles:

1. **Entry Point Generation**: The `export!()` macro generates the C ABI entry points (`waterui_init`, `waterui_app`) that native backends call to initialize and run Rust applications.

2. **Type Conversion**: Implements `IntoFFI` and `IntoRust` traits to safely convert between Rust types (views, reactive values, colors, fonts) and C-compatible representations that can cross the FFI boundary.

3. **C Header Generation**: Uses `cbindgen` to automatically generate `waterui.h`, which is consumed by Swift (Apple backend) and Kotlin/JNI (Android backend) to understand the FFI contract.

The FFI layer is designed to work in `no_std` environments and minimizes unsafe code through carefully designed abstractions.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui = "0.1"
waterui-ffi = "0.1"
```

For applications, you typically only need the `export!()` macro - all other FFI details are handled internally by WaterUI.

## Quick Start

Every WaterUI application uses the FFI layer to expose itself to native platforms:

```rust
use waterui::prelude::*;
use waterui::app::App;

// Your application entry point
pub fn app(env: Environment) -> App {
    App::new(main, env)
}

fn main() -> impl View {
    text("Hello, WaterUI!")
}

// This macro generates waterui_init() and waterui_app() FFI entry points
waterui_ffi::export!();
```

The `export!()` macro expands to:

```rust
#[no_mangle]
pub unsafe extern "C" fn waterui_init() -> *mut WuiEnv {
    // Initialize runtime, logging, executors
    let env = waterui::Environment::new();
    env.into_ffi()
}

#[no_mangle]
pub unsafe extern "C" fn waterui_app(env: *mut WuiEnv) -> WuiApp {
    let env = env.into_rust();
    let app = app(env);  // Call your app() function
    app.into_ffi()
}
```

Native backends then call these functions to initialize and retrieve the root view tree.

## Core Concepts

### FFI Conversion Traits

The crate defines two fundamental traits for crossing the FFI boundary:

**`IntoFFI`** - Converts Rust types to FFI-compatible representations:

```rust
pub trait IntoFFI: 'static {
    type FFI: 'static;
    fn into_ffi(self) -> Self::FFI;
}

// Example: Converting a String to a C-compatible byte array
impl IntoFFI for Str {
    type FFI = WuiStr;
    fn into_ffi(self) -> Self::FFI {
        WuiStr(WuiArray::new(self))
    }
}
```

**`IntoRust`** - Safely converts FFI types back to Rust types:

```rust
pub trait IntoRust {
    type Rust;
    unsafe fn into_rust(self) -> Self::Rust;
}

// Example: Converting C byte array back to String
impl IntoRust for WuiStr {
    type Rust = Str;
    unsafe fn into_rust(self) -> Self::Rust {
        let bytes = unsafe { self.0.into_rust() };
        unsafe { Str::from_utf8_unchecked(bytes) }
    }
}
```

### Opaque Types

For types where the internal structure isn't relevant to native code, use the `OpaqueType` trait:

```rust
impl OpaqueType for Environment {}

// Automatically implements:
// - IntoFFI converting to *mut WuiEnv
// - IntoRust converting from *mut WuiEnv
```

This pattern is used for `Environment`, `AnyView`, `Binding<T>`, and `Computed<T>`, which native code treats as opaque pointers.

### Type Identification

The FFI layer uses `WuiTypeId` for O(1) type identification across the FFI boundary:

```rust
#[repr(C)]
pub struct WuiTypeId {
    pub low: u64,
    pub high: u64,
}
```

In normal builds, this wraps Rust's `TypeId`. In hot reload builds (with `waterui_hot_reload_lib` cfg), it uses a 128-bit FNV-1a hash of the type name, ensuring type IDs remain stable across dylib reloads.

Native backends use type IDs to determine which view type they're rendering:

```swift
let viewId = waterui_view_id(view)

if viewId == waterui_text_id() {
    let textConfig = waterui_force_as_text(view)
    return Text(textConfig.content.get())
} else if viewId == waterui_button_id() {
    // ...
}
```

### View Rendering Protocol

Native backends traverse the view tree using these core functions:

1. **`waterui_view_id(view)`** - Get the 128-bit type ID
2. **`waterui_view_body(view, env)`** - Expand composite views into their body
3. **`waterui_force_as_<type>(view)`** - Downcast to specific view type (Button, Text, etc.)
4. **`waterui_view_stretch_axis(view)`** - Query layout behavior

Composite views (user-defined) are recursively expanded via `body()`. Leaf views (Text, Button, Image) are downcast and mapped directly to native widgets.

## Reactive System FFI

WaterUI's reactive primitives (`Binding`, `Computed`) cross the FFI boundary with full functionality:

### Binding (Read/Write Reactive State)

```rust
// Rust side
let counter = Binding::int(42);

// FFI functions generated by ffi_binding! macro:
// - waterui_read_binding_i32(binding) -> i32
// - waterui_set_binding_i32(binding, value)
// - waterui_watch_binding_i32(binding, watcher) -> guard
// - waterui_drop_binding_i32(binding)
```

Native code can read, write, and subscribe to changes:

```swift
let binding: UnsafeMutablePointer<WuiBinding<Int32>>
let value = waterui_read_binding_i32(binding)
waterui_set_binding_i32(binding, value + 1)

// Watch for changes
let guard = waterui_watch_binding_i32(binding) { newValue, metadata in
    print("Counter changed to \(newValue)")
}
```

### Computed (Read-Only Derived State)

```rust
// Rust side
let doubled = counter.map(|n| n * 2);

// FFI functions generated by ffi_computed! macro:
// - waterui_read_computed_i32(computed) -> i32
// - waterui_watch_computed_i32(computed, watcher) -> guard
// - waterui_clone_computed_i32(computed) -> computed
// - waterui_drop_computed_i32(computed)
```

Computed values can also be created from native code using `waterui_new_computed_<type>()`, enabling native-driven reactivity.

## Architecture

### Data Flow

```
┌─────────────────────────────────────────────────────┐
│ Rust Application (waterui)                         │
│ - View tree definition                             │
│ - Reactive state (Binding, Computed)               │
│ - Business logic                                    │
└─────────────────┬───────────────────────────────────┘
                  │ .into_ffi()
                  ▼
┌─────────────────────────────────────────────────────┐
│ FFI Layer (waterui-ffi)                            │
│ - Entry points: waterui_init(), waterui_app()      │
│ - Type conversion: IntoFFI, IntoRust               │
│ - View traversal: waterui_view_id(), _body()       │
│ - Reactive primitives: Binding, Computed FFI       │
└─────────────────┬───────────────────────────────────┘
                  │ C ABI (waterui.h)
                  ▼
┌─────────────────────────────────────────────────────┐
│ Native Backend (Swift/Kotlin)                      │
│ - Apple: SwiftUI views (Text, Button, HStack...)  │
│ - Android: Jetpack Compose (@Composable)          │
│ - Maps Rust views to platform widgets             │
└─────────────────────────────────────────────────────┘
```

### Component FFI Modules

The `components/` directory contains FFI bindings for each UI component category:

- **`layout`** - HStack, VStack, ZStack, ScrollView, Spacer
- **`button`** - Button with styles (plain, bordered, link)
- **`text`** - Text rendering, fonts, styled text
- **`form`** - TextField, SecureField, Toggle, Slider, Picker
- **`navigation`** - NavigationStack, TabView, NavigationLink
- **`media`** - Image, Video, Audio, LivePhoto
- **`list`** - List, ForEach, LazyVStack
- **`table`** - Table with columns and rows
- **`progress`** - ProgressView, ProgressIndicator
- **`gpu_surface`** - High-performance wgpu rendering surface

Each module defines:
1. C-compatible struct representations (e.g., `WuiButton`)
2. `IntoFFI` implementations for converting Rust view configs
3. FFI view macros generating type ID and downcast functions

### Helper Macros

The crate provides several code generation macros to reduce boilerplate:

**`opaque!(Name, RustType, ident)`** - Generate opaque pointer FFI for a type:
```rust
opaque!(WuiEnv, waterui::Environment, env);
// Generates: WuiEnv wrapper, IntoFFI, IntoRust, waterui_drop_env()
```

**`ffi_view!(RustView, FFIStruct, ident)`** - Generate view FFI functions:
```rust
ffi_view!(ButtonConfig, WuiButton, button);
// Generates: waterui_button_id(), waterui_force_as_button()
```

**`ffi_reactive!(Type, FFIType, ident)`** - Generate binding + computed FFI:
```rust
ffi_reactive!(i32, i32, i32);
// Generates: read/write/watch/drop for both Binding<i32> and Computed<i32>
```

**`into_ffi!{}`** - Derive `IntoFFI` for structs and enums:
```rust
into_ffi! {
    ButtonStyle,
    pub enum WuiButtonStyle {
        Plain, Bordered, Link,
    }
}
```

## Examples

### Creating a New FFI View Type

To add FFI support for a new view component:

```rust
// 1. Define the Rust view config (in components/controls/src/rating.rs)
pub struct RatingConfig {
    pub value: Computed<f32>,
    pub max: f32,
    pub color: Color,
}

// 2. Add FFI bindings (in ffi/src/components/controls.rs)
use crate::{IntoFFI, reactive::WuiComputed, color::WuiColor};

into_ffi! {
    RatingConfig,
    pub struct WuiRating {
        value: *mut WuiComputed<f32>,
        max: f32,
        color: *mut WuiColor,
    }
}

ffi_view!(RatingConfig, WuiRating, rating);

// 3. Regenerate waterui.h
// cargo run --bin generate_header --features cbindgen --manifest-path ffi/Cargo.toml

// 4. Implement in Swift (backends/apple/Sources/WaterUI/Views/Rating.swift)
// if viewId == waterui_rating_id() {
//     let config = waterui_force_as_rating(view)
//     return RatingView(config: config)
// }
```

### Implementing Custom Reactive Properties

```rust
// Generate FFI for a custom type in reactive state
use waterui_color::Color;

ffi_reactive!(Color, *mut WuiColor, color);
// Now Binding<Color> and Computed<Color> can cross FFI
```

### Creating Native-Controlled Computed Values

```rust
// Swift side creates a computed that Rust can read
let computed = waterui_new_computed_i32(
    dataPtr,
    { ptr in return getCurrentValue(ptr) },
    { ptr, watcher in return watchValue(ptr, watcher) },
    { ptr in cleanup(ptr) }
)

// Pass to Rust view
let text = waterui_text(computed)
```

## C Header Generation

The crate includes a `generate_header` binary that uses `cbindgen` to produce `waterui.h`:

```bash
cargo run --bin generate_header --features cbindgen --manifest-path ffi/Cargo.toml
```

This generates the C header and automatically copies it to:
- `backends/apple/Sources/CWaterUI/include/waterui.h` (SwiftUI backend)
- `backends/android/runtime/src/main/cpp/waterui.h` (Android backend)

The header is checked into version control, and CI verifies it's always up-to-date with the Rust code.

## Native Backend Render Pipeline

Native backends (Android, Apple, etc.) must follow a specific initialization sequence when rendering WaterUI views.

### Required Sequence

```
┌─────────────────────────────────────────────────────────────────────┐
│ 1. waterui_init()                                                   │
│    - Initializes panic hooks and global executors                   │
│    - Returns an Environment pointer                                 │
│    - MUST be called first before any other waterui_* functions      │
├─────────────────────────────────────────────────────────────────────┤
│ 2. waterui_env_install_theme(env, colors..., fonts...)              │
│    - Injects native theme colors and fonts as reactive signals      │
│    - Reads system/Material Design colors and passes them to Rust    │
│    - Optional but recommended for proper theming                    │
├─────────────────────────────────────────────────────────────────────┤
│ 3. waterui_app(env)                                                 │
│    - Creates the application from user's app(env) function          │
│    - Returns WuiApp with windows and environment                    │
│    - MUST be called AFTER waterui_init() and theme installation     │
├─────────────────────────────────────────────────────────────────────┤
│ 4. Render Loop (for each view)                                      │
│    a. waterui_view_id(view) → Get the type ID                       │
│    b. Check if it's a "raw view" (Text, Button, etc.)               │
│       - If raw: waterui_force_as_*(view) → Extract native data      │
│       - If composite: waterui_view_body(view, env) → Get body view  │
│    c. Render the native widget or recurse into body                 │
└─────────────────────────────────────────────────────────────────────┘
```

### Raw Views vs Composite Views

WaterUI distinguishes between two kinds of views:

- **Raw Views**: Leaf components that map directly to native widgets. Examples: `Text`, `Button`, `Color`, `TextField`, `Toggle`, `Slider`, `Stepper`, `Progress`, `Spacer`, `Picker`, `ScrollView`.

- **Composite Views**: User-defined views that have a `body()` method returning other views. When you encounter a view that isn't in the raw view registry, call `waterui_view_body(view, env)` to get its body and continue rendering recursively.

## Features

- **`std`** (default) - Enable standard library support
- **`cbindgen`** - Required for the `generate_header` binary

## API Overview

### Core Entry Points
- `waterui_init()` - Initialize runtime and return Environment
- `waterui_app(env)` - Create application from user's app() function
- `waterui_env_new()` - Create new Environment (alternative to init)
- `waterui_clone_env(env)` - Clone Environment for child contexts

### View Traversal
- `waterui_view_id(view)` - Get type ID as 128-bit value
- `waterui_view_body(view, env)` - Expand composite view to its body
- `waterui_view_stretch_axis(view)` - Query layout stretch behavior
- `waterui_empty_anyview()` - Create empty view
- `waterui_anyview_id()` - Get AnyView type ID

### Type Downcasting
- `waterui_force_as_<type>(view)` - Downcast to specific view type
- `waterui_<type>_id()` - Get type ID for comparison

### Reactive Primitives
- `waterui_read_binding_<type>(binding)` - Read current value
- `waterui_set_binding_<type>(binding, value)` - Update value
- `waterui_watch_binding_<type>(binding, watcher)` - Subscribe to changes
- `waterui_read_computed_<type>(computed)` - Read derived value
- `waterui_watch_computed_<type>(computed, watcher)` - Subscribe to changes
- `waterui_new_computed_<type>(...)` - Create native-controlled computed
- `waterui_drop_binding_<type>(binding)` - Cleanup binding
- `waterui_drop_computed_<type>(computed)` - Cleanup computed

### Memory Management
- `waterui_drop_<type>(ptr)` - Free resource of given type
- `waterui_drop_retain(retain)` - Drop retained value

## Safety Considerations

The FFI layer involves extensive `unsafe` code by necessity:

1. **Pointer Validity**: Callers must ensure pointers from FFI functions remain valid for their usage
2. **Ownership Transfer**: Functions taking `*mut T` typically take ownership and will free the memory
3. **Thread Safety**: `waterui_init()` must be called once on the main thread only
4. **Type Downcasting**: `waterui_force_as_*()` functions assume the view type matches

Native backends are responsible for:
- Properly managing the lifetimes of FFI pointers
- Calling drop functions when resources are no longer needed
- Not using pointers after they've been consumed

## Performance

The FFI layer is designed for zero-overhead abstraction:

- **Type IDs**: O(1) comparison (128-bit integer equality)
- **View Traversal**: Single pointer dereference per level
- **Reactive Updates**: Direct function pointer callbacks, no allocation
- **Arrays**: Zero-copy views into Rust data via vtable-based slicing

The reactive system uses reference counting (`Rc`) on the Rust side and lets native code subscribe via lightweight watcher callbacks.

## Development Workflow

When adding a new view type to WaterUI:

1. Define the Rust view struct in the appropriate component crate
2. Add FFI bindings in `ffi/src/components/<module>.rs`
3. Regenerate the C header: `cargo run --bin generate_header --features cbindgen --manifest-path ffi/Cargo.toml`
4. Implement the native renderer in Swift (`backends/apple`) and Kotlin (`backends/android`)
5. Update tests to verify FFI contract

The workflow ensures Rust, C header, and native backends stay synchronized.

## License

MIT
