# waterui-ffi

This crate provides the Foreign Function Interface (FFI) for WaterUI.

## C Header

The C header file `waterui.h` is used by downstream backends (like Swift and Java/Kotlin) to interact with the Rust core.

### Updating the Header

The `waterui.h` header is generated using `cbindgen` and is checked into version control. To update it after making changes to the FFI API, run the following command from the root of the `waterui` repository:

```bash
cargo run --bin generate_header --features cbindgen --manifest-path ffi/Cargo.toml
```

The CI will verify that the header is up-to-date.

## How applications use the FFI

Every WaterUI application is just a normal Rust crate (it has its own `Cargo.toml` and `src/lib.rs`). That crate:

1. Depends on `waterui` to define the application logic (`app(env: Environment) -> App` entry point).
2. Depends on `waterui-ffi` for the `waterui_ffi::export!()` macro, which expands to the `#[no_mangle] extern "C"` entry points `waterui_init` and `waterui_app`.
3. Re-exports those entry points so any native shell (Android, Apple, web) can call straight into the Rust code.

The CLI now exposes `water build <platform>` to produce these native artifacts.
When you run `water run android`/`water package android`, it invokes
`water build android` before Gradle so Cargo builds **the project crate** (not
the helper crate) for every requested Android ABI. The built `.so` is copied
into `android/app/src/main/jniLibs/` and loaded by the Android runtime, which
expects the exported `waterui_init`/`waterui_app` symbols documented above.
Apple backends follow the same pattern: Xcode runs the tiny wrapper script that
executes `water build apple`, links the resulting static library, and calls the
exported functions via `waterui.h`.

Because the project crate owns the exports, any change you make to `init()`/`main()` is automatically available to every platform shell—there is no need to edit `waterui-ffi` itself, and you should never build `waterui-ffi` as a standalone artifact.

## Native Backend Render Pipeline

Native backends (Android, Apple, etc.) must follow a specific initialization sequence when rendering WaterUI views. The order is critical because some operations depend on the Rust runtime being properly initialized.

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
│    a. waterui_view_id(view) → Get the type name                     │
│    b. Check if it's a "raw view" (Text, Button, Color, etc.)        │
│       - If raw: waterui_force_as_*(view) → Extract native data      │
│       - If composite: waterui_view_body(view, env) → Get body view  │
│    c. Render the native widget or recurse into body                 │
└─────────────────────────────────────────────────────────────────────┘
```

### Raw Views vs Composite Views

WaterUI distinguishes between two kinds of views:

- **Raw Views**: Leaf components that map directly to native widgets. Examples: `Text`, `Button`, `Color`, `TextField`, `Toggle`, `Slider`, `Stepper`, `Progress`, `Spacer`, `Picker`, `ScrollView`, `RendererView`.

- **Composite Views**: User-defined views that have a `body()` method returning other views. When you encounter a view that isn't in the raw view registry, call `waterui_view_body(view, env)` to get its body and continue rendering recursively.

### Type ID Functions

Each raw view type has a corresponding ID function:

| Function                         | View Type      |
|----------------------------------|----------------|
| `waterui_text_id()`              | Text           |
| `waterui_button_id()`            | Button         |
| `waterui_color_id()`             | Color          |
| `waterui_text_field_id()`        | TextField      |
| `waterui_toggle_id()`            | Toggle         |
| `waterui_slider_id()`            | Slider         |
| `waterui_stepper_id()`           | Stepper        |
| `waterui_progress_id()`          | Progress       |
| `waterui_spacer_id()`            | Spacer         |
| `waterui_picker_id()`            | Picker         |
| `waterui_scroll_view_id()`       | ScrollView     |
| `waterui_dynamic_id()`           | Dynamic        |
| `waterui_layout_container_id()`  | LayoutContainer|
| `waterui_fixed_container_id()`   | FixedContainer |
| `waterui_renderer_view_id()`     | RendererView   |
| `waterui_empty_id()`             | EmptyView      |

### Common Pitfalls

1. **Calling `waterui_app()` before `waterui_init()`**: This causes reactive signals to be created without a properly initialized executor, leading to silent failures where components don't render or update correctly.

2. **Not installing theme before rendering**: While optional, failing to call `waterui_env_install_theme()` means the Rust side will use fallback colors/fonts, which may not match the native platform's appearance.

3. **Memory leaks**: Always call the corresponding `waterui_drop_*()` functions when disposing of native handles (views, environments, bindings, etc.).
