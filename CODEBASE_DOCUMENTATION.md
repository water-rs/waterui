# WaterUI Codebase Documentation

## Table of Contents

1. [Overview](#overview)
2. [Workspace Layout](#workspace-layout)
3. [Build & Verification Commands](#build--verification-commands)
4. [Architectural Foundations](#architectural-foundations)
   1. [Declarative View System](#declarative-view-system)
   2. [Environment & Plugin Pipeline](#environment--plugin-pipeline)
   3. [Reactive State Management](#reactive-state-management)
   4. [Configurable & Raw Views](#configurable--raw-views)
5. [Component Libraries](#component-libraries)
   1. [Layout](#layout)
   2. [Text](#text)
   3. [Form](#form)
   4. [Media](#media)
   5. [Navigation](#navigation)
   6. [Graphics](#graphics)
6. [Utility Libraries](#utility-libraries)
7. [Backend Implementations](#backend-implementations)
8. [FFI Interface](#ffi-interface)
9. [Development Workflow & Standards](#development-workflow--standards)
10. [Roadmap & Future Work](#roadmap--future-work)

---

## Overview

WaterUI is a modern, cross-platform UI framework implemented in Rust. Applications declare their UI using the `waterui` crate and render through platform backends without touching the DOM or a virtual tree. The framework focuses on:

- **Declarative Composition:** Views implement the [`View`](core/src/view.rs) trait and compose recursively.
- **Fine-grained Reactivity:** Integration with the [`nami`](https://docs.rs/nami) library powers bindings (`Binding`) and derived values (`Computed`).
- **Native Rendering:** Each platform backend consumes FFI-friendly view descriptions and renders with native toolkits.
- **`no_std` Compatibility:** Crates such as `waterui-core`, `waterui-str`, and `ffi` operate without the Rust standard library unless the `std` feature is explicitly enabled.

---

## Workspace Layout

The repository is a Cargo workspace anchored at `waterui/Cargo.toml`.

```text
waterui/
├── Cargo.toml           # Workspace configuration and lint settings
├── core/                # `waterui-core` crate (core traits, env, plugins)
├── components/          # Feature-specific UI primitives
│   ├── form/            # Input controls and derive helper
│   ├── graphics/        # Canvas, renderer bridge, drawing context
│   ├── layout/          # Layout containers, spacing, scrolling
│   ├── media/           # Image and media views
│   ├── navigation/      # Navigation views built on `raw_view!`
│   └── text/            # Text primitives and macros
├── utils/               # Shared utilities (`str`, `color`, `url`)
├── backends/            # Platform renderers (android, apple, tui, web, hydrolysis)
│   └── hydrolysis/      # Hydrolysis self-drawn engine
├── window/              # Window system abstractions
├── ffi/                 # Cross-language interface definitions
├── derive/              # Procedural macros for component derive impls
├── cli/                 # Developer tooling
└── demo/                # Example application wiring a backend
```

Workspace membership and shared lint settings are defined in [`Cargo.toml`](Cargo.toml). The workspace uses Rust edition 2024 with a minimum supported Rust version (MSRV) of 1.87.

---

## Build & Verification Commands

WaterUI enforces a strict "tests and linters first" workflow. Common tasks can be run from the repository root:

```bash
# Format checks
cargo fmt --all -- --check

# Clippy linting across every crate
cargo clippy --all-targets --all-features --workspace -- -D warnings

# Build and test the entire workspace
cargo build --all-features --workspace
cargo test --all-features --workspace

# Generate documentation
cargo doc --all-features --no-deps --workspace

# Memory safety validation for `waterui-str`
cargo +nightly miri test -p waterui-str
```

These commands align with the scripts referenced in `GEMINI.md` and `CLAUDE.md` and match the workspace configuration.

---

## Architectural Foundations

### Declarative View System

The `waterui-core` crate defines the [`View`](core/src/view.rs) trait, which every composable UI element implements:

```rust
pub trait View: 'static {
    fn body(self, _env: &Environment) -> impl View;
}
```

Any type returning another `View` can be treated as a view (e.g., functions, `Result`, and `Option` implement `View`). `AnyView` performs type erasure, enabling heterogeneous collections without sacrificing ergonomics.

### Environment & Plugin Pipeline

The [`Environment`](core/src/env.rs) acts as a type-indexed map shared down the view tree. It supports chained insertion via `with`, plugin installation via `install`, and hook registration (`insert_hook`) to intercept [`ViewConfiguration`](core/src/view.rs) rendering. Plugins implement [`Plugin`](core/src/plugin.rs) and can provision global resources, theming, or hooks without modifying downstream code.

### Reactive State Management

WaterUI re-exports `Binding`, `Computed`, and related traits from `nami` (`waterui::reactive`). Bindings propagate changes through the view tree, and helper components such as [`Dynamic`](core/src/components/dynamic.rs) and macros like [`text!`](components/text/src/macros.rs) translate reactive data into views. The `text!` macro produces a `Text` view backed by a `nami::s!` signal, ensuring updates remain reactive.

### Configurable & Raw Views

Many components follow the **Configurable View** pattern: a public builder struct exposes a fluent API and converts into a configuration object that implements [`ViewConfiguration`](core/src/view.rs). Configuration structs are trivially serializable into the FFI layer.

Primitive elements that require backend cooperation use the [`raw_view!`](core/src/macros.rs) macro. Raw views expose metadata to the renderer; examples include `FixedContainer`, `ScrollView`, `Spacer`, and backend bridge types such as [`RendererView`](components/graphics/src/renderer_view.rs).

---

## Component Libraries

### Layout

The `waterui-layout` crate covers containers, stacks, scrolling, and sizing. Components such as [`Frame`](components/layout/src/frame.rs) offer a fluent builder API for constraints (`width`, `height`, `min_width`, etc.) and render through layout-aware raw views (`FixedContainer`). Other layout primitives include stacks (`VStack`, `HStack`, `ZStack`), `Spacer`, and `ScrollView`.

### Text

`waterui-text` provides the `Text` view, attributed string support, typography helpers, and the `text!` macro for reactive formatting. `Text::new`, `Text::display`, and builder methods like `font` and `size` configure typography while keeping data reactive.

### Form

`waterui-form` contains input controls such as `TextField`, `Toggle`, `Slider`, `Stepper`, and picker views. The crate exposes builder APIs and relies on bindings for two-way data flow. A companion `waterui-form-derive` crate lives in `components/form/derive` for declarative form bindings.

### Media

`waterui-media` exposes image and video primitives. Components such as `Image` and `AsyncImage` translate media metadata into backend requests while maintaining reactive updates.

### Navigation

`waterui-navigation` implements navigation containers (`NavigationLink`, tab-style views, etc.) using `raw_view!` to bridge routing metadata to backends. Navigation views manage destination payloads and rely on environment state for navigation stacks.

### Graphics

`waterui-graphics` offers custom drawing primitives. [`Canvas`](components/graphics/src/canvas.rs) and [`GraphicsContext`](components/graphics/src/context.rs) let Rust code draw using CPU buffers provided by [`RendererView`](components/graphics/src/renderer_view.rs). Optional WGPU support extends the renderer surface for GPU-backed workflows.

---

## Utility Libraries

- **`waterui-str`** (`utils/str`): Implements the [`Str`](utils/str/src/lib.rs) type, which stores either static strings or reference-counted owned data. It supports `no_std`, efficient cloning, and comprehensive conversion APIs.
- **`waterui-color`** (`utils/color`): Provides color manipulation utilities and exposes a `Color` raw view for declarative color descriptions.
- **`waterui-url`** (`utils/url`): Supplies URL parsing and handling tailored for UI navigation contexts.

---

## Backend Implementations

- **Android** – Compose runtime hosted in `backends/android/runtime`. The CLI injects `WATERUI_HOT_RELOAD_HOST/PORT` via intent extras and performs `adb reverse`. The Kotlin code’s only hot reload duties are calling `configureHotReloadEndpoint` and pointing `configureHotReloadDirectory` at `codeCacheDir`; all WebSocket, `.so` download, and `dlopen` logic lives in Rust’s `Hotreload` view.
- **Web** – The experimental WASM backend recreates `pkg/app.js` on rebuilds. Hot reload is handled by `cli/src/templates/web/main.js`, which listens on `/hot-reload-web` and refreshes the page when the CLI broadcasts a change.
- **TUI** – The terminal backend in `backends/tui` provides a renderer (`TuiApp`, `Renderer`, `Terminal`) but is not yet wired into the CLI for hot reload or scaffolding. TUI work is intentionally paused until the core native backends are complete.

Backend crates receive FFI structs and render native widgets:

- **`backends/android`**: Android shell integrating with Gradle; documentation and status live in `PLAN.md` and `IMPLEMENTATION_STATUS.md`.
- **`backends/apple`**: Placeholder for Apple platforms (structure reserved for SwiftUI integration).
- **`backends/tui`**: Terminal UI backend scaffolding (`Cargo.toml`, roadmap `PLAN.md`).
- **`backends/web`**: WebAssembly backend using `wasm-bindgen`, `web-sys`, and `js-sys` for DOM interaction.

Each backend uses Hydrolysis—the project’s single self-drawn renderer—and the platform-agnostic window abstractions in `window`.

---

## Hot Reload Integration Notes

All hot reload logic is implemented inside Rust (`waterui::hot_reload::Hotreload`). Native shells (Android, macOS, simulator targets) have only two responsibilities:

1. **Endpoint injection** – call `waterui_configure_hot_reload_endpoint(host, port)` (exposed as `configureHotReloadEndpoint` in Kotlin/Swift). This tells the Rust client which `ws://host:port/hot-reload-native` endpoint the CLI is serving so it can stream updated shared libraries. On Android emulators/devices, the CLI performs `adb reverse tcp:PORT PORT`, then passes `host=127.0.0.1` through the launch intent.
2. **Writable/executable directory** – call `waterui_configure_hot_reload_directory(path)` when the default temp directory is not writable/executable (e.g., Android). Point it at `context.codeCacheDir` or an equivalent cache folder so the Rust runtime can write updated `.so` files before `dlopen`.

If a runtime is linked against an older Rust crate that does not yet export `waterui_configure_hot_reload_directory`, the JNI bridge treats it as optional: the call becomes a no-op and Rust falls back to its built-in temp directory.

In all cases the host platform should **not** create its own WebSocket loop or call `dlopen` directly—those steps remain in Rust to keep the policy consistent across macOS, iOS simulator, Android, and future targets.

---

## FFI Interface

The [`ffi`](ffi/src/lib.rs) crate defines traits such as `IntoFFI`, `IntoRust`, and `OpaqueType` for bridging Rust types into stable C ABI representations. The exported `waterui_init`/`waterui_main` functions bootstrap the runtime, initialize executors, and convert Rust views (`AnyView`) into opaque pointers that native shells consume. This crate is `no_std`, enabling integration with restricted environments.

---

## Development Workflow & Standards

- **Lint Policy:** Workspace-wide lint levels are declared in [`Cargo.toml`](Cargo.toml) under `[workspace.lints]`, promoting documentation completeness and strict Clippy checks.
- **Testing Strategy:** `cargo test --all-features --workspace` exercises unit and integration tests across crates. `waterui-str` includes extensive runtime and memory-safety coverage validated with `cargo +nightly miri test -p waterui-str`.
- **Formatting:** Rustfmt is enforced via CI and local `cargo fmt --all -- --check` runs.
- **Contribution Practices:** Prefer builder-style APIs for configurable views, use `text!` rather than `format!` for reactive strings, and choose between composite widgets and primitive components based on whether backend cooperation is required. Components should fail fast on invalid input and rely on unit tests to enforce invariants.

---

## Roadmap & Future Work

Planned efforts focus on completing backend implementations (Android, Swift, TUI), enhancing async support through executor integration, expanding the plugin system, and stabilizing APIs for broader adoption. See per-backend `PLAN.md` files and `ROADMAP.md` for the latest milestones.
