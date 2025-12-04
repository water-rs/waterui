---
name: waterui-native-bindings
description: Implement FFI bindings for WaterUI components across Rust, Apple Swift, and Android Kotlin/JNI layers. Use when adding new component types, metadata wrappers, or fixing native binding errors (UnsatisfiedLinkError, missing symbols, type mismatches).
---

# WaterUI Native Bindings

## Architecture

```
Rust Core → Rust FFI (ffi/src/) → cbindgen → waterui.h
                                      ├── Apple (Swift + C)
                                      └── Android (Kotlin + JNI + C++)
```

## Layout Contract

**Rust owns layout, native owns rendering.** Native components must:

1. **Measure based on content** - Report intrinsic size via `sizeThatFits` (Apple) or `onMeasure` (Android)
2. **Accept placement from Rust** - Rust calls `place(x, y, width, height)` to position the view
3. **Use native UI components** - Render with platform-native widgets (UIKit/AppKit/Android Views)
4. **Customize behavior freely** - Handle gestures, animations, styling within the component

```
Rust Layout Engine          Native Component
      │                           │
      ├── sizeThatFits? ─────────►│ (measure content)
      │◄──────────────── size ────┤
      │                           │
      ├── place(x,y,w,h) ────────►│ (accept position)
      │                           │
      └── (repeat per frame) ─────┘
```

Native components should NOT fight the layout system - measure honestly, accept placement.

**Metadata never affects layout.** Metadata wrappers (gestures, focus, background, etc.) are transparent to the layout system - they pass through the child's size unchanged. Only the wrapped content determines layout.

## Building Native Components

### Rust Side: Define with `NativeView` trait

```rust
pub struct MyComponent { /* fields */ }

impl NativeView for MyComponent {
    // Declare stretch behavior (how component fills available space)
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None        // Fixed size, no stretching
        // StretchAxis::Horizontal // Fills width, fixed height
        // StretchAxis::Vertical   // Fixed width, fills height
        // StretchAxis::Both       // Fills both dimensions
    }
}
```

### Native Side: Render with Platform Views

**Apple (Swift):**

```swift
final class WuiMyComponent: WuiBaseView, WuiComponent {
    static var rawId: WuiTypeId { waterui_my_component_id() }

    required init(anyview: OpaquePointer, env: WuiEnvironment) {
        super.init(frame: .zero)
        let data = waterui_force_as_my_component(anyview)

        // Use native UIKit/AppKit views
        let label = UILabel()  // or NSTextField for macOS
        addSubview(label)
    }

    // Report intrinsic size for Rust layout
    override func sizeThatFits(_ size: CGSize) -> CGSize {
        return label.sizeThatFits(size)
    }
}
```

**Android (Kotlin):**

```kotlin
private val myComponentRenderer = WuiRenderer { context, node, env, registry ->
    val data = NativeBindings.waterui_force_as_my_component(node.rawPtr)

    // Use native Android views
    val textView = TextView(context).apply {
        text = "Hello"
    }

    // Set stretch axis tag for layout system
    textView.setTag(TAG_STRETCH_AXIS, StretchAxis.NONE)

    textView.disposeWith { /* cleanup */ }
    textView
}
```

### StretchAxis Values

| Value        | Behavior                         | Use Case                   |
| ------------ | -------------------------------- | -------------------------- |
| `None`       | Fixed size from content          | Text, icons, buttons       |
| `Horizontal` | Fills width, height from content | Text fields, progress bars |
| `Vertical`   | Fills height, width from content | Vertical dividers          |
| `Both`       | Fills all available space        | Backgrounds, containers    |

## Quick Reference

| Layer       | Key Files                                                          |
| ----------- | ------------------------------------------------------------------ |
| FFI         | `ffi/src/lib.rs`, `ffi/src/macros.rs`                              |
| Apple       | `backends/apple/Sources/WaterUI/Components/`, `Core/AnyView.swift` |
| Android     | `backends/android/runtime/src/main/java/dev/waterui/android/`      |
| Android JNI | `backends/android/runtime/src/main/cpp/waterui_jni.cpp`            |

## Adding a New Component

### 1. Rust FFI (`ffi/src/lib.rs`)

```rust
#[repr(C)]
pub struct WuiFooData { pub field: i32 }

pub type WuiMetadataFoo = WuiMetadata<WuiFooData>;
ffi_metadata!(path::to::Foo, WuiMetadataFoo, foo);
```

Generate header: `cargo run --bin generate_header --features cbindgen`

### 2. Apple Backend

Create `backends/apple/Sources/WaterUI/Components/WuiFoo.swift`:

```swift
@MainActor
final class WuiFoo: WuiBaseView, WuiComponent {
    static var rawId: CWaterUI.WuiTypeId { waterui_foo_id() }
    required init(anyview: OpaquePointer, env: WuiEnvironment) {
        let data = waterui_force_as_foo(anyview)
        super.init(frame: .zero)
        // Implementation...
        disposeBag.append { waterui_drop_foo(data.ptr) }
    }
}
```

Register in `AnyView.swift`: `registerComponent(WuiFoo.self)`

### 3. Android Backend

See [references/android-jni.md](references/android-jni.md) for complete step-by-step.

**Quick checklist:**

1. `FfiStructs.kt` - Add data class
2. `WatcherJni.kt` - Add `external fun` declarations
3. `NativeBindings.kt` - Add wrapper functions
4. `waterui_jni.cpp` - Add to symbol table + implement JNI functions
5. `components/FooComponent.kt` - Create renderer
6. `RenderRegistry.kt` - Register component

## Reactive Values

**Swift:**

```swift
let computed = WuiComputed<T>(ptr, env: env)
computed.watch { value in /* react */ }

let binding = WuiBinding<Bool>(ptr, env: env)
binding.observe { value in /* react */ }
binding.set(true)
```

**Kotlin:**

```kotlin
val computed = WuiComputed.resolvedColor(ptr, env)
computed.observe { value -> /* react */ }
computed.close()

val binding = WuiBinding.bool(ptr, env)
binding.observe { value -> /* react */ }
binding.set(true)
binding.current()
binding.close()
```

## Build Commands

```bash
# Generate C header
cd ffi && cargo run --bin generate_header --features cbindgen

# Apple
cd backends/apple && swift build

# Android
cd backends/android && ./gradlew :runtime:assembleDebug
```

## Troubleshooting

| Error                  | Cause                  | Fix                                             |
| ---------------------- | ---------------------- | ----------------------------------------------- |
| `UnsatisfiedLinkError` | Missing JNI symbol     | Add to `WATCHER_SYMBOL_LIST` in waterui_jni.cpp |
| Swift "Cannot find X"  | Header outdated        | Regenerate + copy waterui.h                     |
| Type mismatch          | Wrong pointer type     | Check header for exact type name                |
| Unit type `()` fails   | C can't represent `()` | Use marker struct: `struct Marker { _m: u8 }`   |

## Adding New Primitive Types

When adding support for a new primitive type (e.g., `f32`, custom struct), update all layers:

### Swift Side

1. **Watcher.swift** - Add watcher creator following existing patterns (see `makeDoubleWatcher`)
2. **Binding.swift** - Add `WuiBinding` extension with read/watch/set/drop functions
3. **Computed.swift** - Add `WuiComputed` extension if needed for computed values

### Android Side

1. **WatcherJni.kt** - Add `external fun` declarations for read/set/drop/watch/create
2. **WuiBinding.kt** - Add factory function in companion object + `WatcherStructFactory` function
3. **WuiComputed.kt** - Add factory function if needed for computed values
4. **waterui_jni.cpp** - Add symbols to `WATCHER_SYMBOL_LIST` + implement JNI functions

### Pattern: Follow Existing Types

Look at how `Double`/`f64` is implemented across all layers and replicate the pattern for your new type. The key functions needed are:

- `read_binding_<type>` / `read_computed_<type>`
- `set_binding_<type>`
- `drop_binding_<type>` / `drop_computed_<type>`
- `watch_binding_<type>` / `watch_computed_<type>`
- `new_watcher_<type>`

## Custom Struct Types (Computed)

For custom struct types like `Video`, `ResolvedColor`, etc.:

1. **FfiStructs.kt** - Add Kotlin data class matching the C struct
2. **WatcherJni.kt** - Add `readComputed<Type>`, `watchComputed<Type>`, `dropComputed<Type>`, `create<Type>Watcher`
3. **WuiComputed.kt** - Add factory function
4. **waterui_jni.cpp** - Convert C struct fields to Java object in JNI function
