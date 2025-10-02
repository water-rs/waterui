# Gemini Code Assistant Context: WaterUI

This document provides a comprehensive overview of the WaterUI project, its architecture, and development conventions to guide the Gemini code assistant in navigating this large codebase.

## 1. Project Overview

WaterUI is a modern, cross-platform UI framework for Rust. Its primary goal is to enable developers to write application logic and UI structure once in Rust and have it render as a true native application on various platforms.

**Core Concepts:**

*   **Declarative & Reactive:** The framework uses a declarative approach to UI definition, inspired by SwiftUI and React. The UI automatically updates when the underlying state changes.
*   **True Native Rendering:** Unlike web-based UI frameworks, WaterUI uses platform-specific backends to render UI components using native widgets (e.g., SwiftUI on Apple platforms, GTK4 on Linux). This ensures maximum performance and a native look-and-feel.
*   **Fine-Grained Reactivity:** State management is powered by the `nami` library, which provides reactive primitives like `Binding` (for mutable state) and `Computed` (for derived state). This allows for efficient updates without a virtual DOM.
*   **Rust Core, Native Shell:** All application logic, state management, and UI composition happens in Rust. The final rendering is delegated to platform backends via a Foreign Function Interface (FFI).

## 2. Architectural Deep Dive

The project is a Cargo workspace with a highly modular architecture designed for scalability and separation of concerns.

*   **`waterui` (Root Crate):** The main public-facing crate. It acts as a facade, curating and re-exporting the most common types and functions from the various sub-crates. Application developers will typically depend on this crate directly.

*   **`waterui-core`:** The absolute foundation of the framework. It is kept minimal and has few dependencies. It defines the essential traits (`View`, `ConfigurableView`), the `Environment` system for context propagation, and integrates the `nami` reactivity library. **All other crates in the workspace depend on `core`**.

*   **`components/*`:** This directory contains the building blocks of the UI. Each sub-crate represents a specific piece of UI functionality.
    *   `waterui-text`: Defines the `Text` component and related typography/font logic.
    *   `waterui-layout`: Provides fundamental layout containers like `vstack`, `hstack`, etc.
    *   `waterui-form`: Contains components for user input like buttons, text fields, etc.
    *   These components are designed to be backend-agnostic.

*   **`kit`:** A higher-level component library. While `components/*` provides the primitives, `kit` aims to provide more complex, pre-styled components (a "UI Kit") built upon the primitives.

*   **`ffi`:** The critical Foreign Function Interface layer. This crate defines the stable C-compatible API that allows the Rust core to communicate with the native backends. It contains `#[repr(C)]` structs that mirror the Rust `...Config` structs and provides functions for the native side to call.

*   **`backends/*`:** Platform-specific renderers. Each backend is a separate crate responsible for implementing the "native shell."
    *   **Backend Contract:** A backend's primary job is to receive the C-structs from the `ffi` layer and translate them into native UI widget operations. It must also handle the application lifecycle, user input events, and send them back to the Rust core.
    *   Examples: `backends/swift` for Apple platforms, `backends/gtk4` for Linux.

*   **`demo`:** A complete, runnable example application that demonstrates how to use the framework with a specific backend (in this case, the `swift` backend for macOS/iOS).

## 3. Core Working Mechanism

The framework's design enables writing UI logic in Rust while leveraging native platform rendering. The data flow from Rust to the screen follows these steps:

1.  **Core Abstraction (`View` Trait):** The foundation is the `View` trait. Every UI element is a `View` that is composed of other `View`s. This creates a declarative and hierarchical UI tree in Rust.

2.  **Reactive System (`nami`):** UI state is managed by the `nami` library. State is held in reactive primitives like `Binding<T>` (for mutable data) and `Computed<T>` (for derived data). When a `Binding` changes, any `Computed` value or UI component that depends on it is automatically marked for an update.

3.  **Data & Behavior Separation (`ConfigurableView`):** A key design pattern is the separation of a component's data from its behavior.
    *   A UI component like `Text` is a `ConfigurableView`. It acts as a builder with methods like `.size()` or `.font()`.
    *   When the view is rendered, its `.config()` method is called to extract a plain data struct (e.g., `TextConfig`). This `...Config` struct implements `ViewConfiguration` and holds all the properties needed for rendering.

4.  **FFI Boundary:** This separation is crucial for cross-language communication.
    *   The pure data `...Config` struct is easily converted into a C-compatible `#[repr(C)]` struct (e.g., `WuiText`) in the `ffi` crate.
    *   This C struct, which is essentially a pointer to a block of memory with a known layout, is passed across the FFI boundary to the native backend.

5.  **Native Rendering:** The platform-specific backend receives the C struct. It reads the data and uses it to create, configure, and display the actual native UI element (e.g., a `UILabel` or `SwiftUI.Text`). When the reactive data updates in Rust, a signal is sent across the FFI to tell the native side to re-read the relevant data and update the native view.

## 4. Building and Running

The project uses standard Cargo commands, which are orchestrated in the CI workflow.

*   **Check formatting:**
    ```bash
    cargo fmt --all -- --check
    ```

*   **Run Linter (Clippy):**
    ```bash
    cargo clippy --all-targets --all-features --workspace -- -D warnings
    ```

*   **Build all crates:**
    ```bash
    cargo build --all-features --workspace
    ```

*   **Run tests:**
    ```bash
    cargo test --all-features --workspace
    ```

*   **Generate documentation:**
    ```bash
    cargo doc --all-features --no-deps --workspace
    ```

## 5. Development Conventions

*   **Workspace Structure:** All code is organized into crates within the Cargo workspace. Dependencies between workspace crates are defined with `package.workspace = true`.

*   **The `ConfigurableView` Pattern:** This is the central pattern for creating new components. A new component should consist of a builder-like `View` struct and a plain data `...Config` struct. This ensures it's extensible and compatible with the FFI layer.

*   **FFI Automation Macros:** The `ffi` crate uses macros to reduce boilerplate when exposing components to the native side.
    *   `ffi_struct!`: Automatically implements the conversion from a Rust `...Config` struct to its C-compatible `Wui...` counterpart.
    *   `ffi_view!`: Creates the necessary FFI functions for a given component.
    *   When adding a new component, you must update the `ffi` crate and use these macros.

*   **Reactivity:** State should be managed using `nami`. Use `binding()` for mutable state and signals (`.map()`, `Computed`) for derived values. This is the idiomatic way to ensure the UI updates automatically.
