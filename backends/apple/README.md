# WaterUI Apple Backend

This repository contains the native Apple backend for the [WaterUI framework](https://github.com/water-rs/waterui), implemented in Swift and leveraging SwiftUI.

## Overview

WaterUI is a modern, cross-platform UI framework implemented in Rust. This backend renders the UI declared in a WaterUI application on Apple platforms (iOS, macOS, iPadOS).

It consumes FFI-friendly view descriptions from the Rust core and renders them using native SwiftUI components. The framework is designed for declarative composition, fine-grained reactivity, and native performance.

## Features

This backend implements a wide range of UI components and features from the WaterUI framework, check implementation status [here](./IMPLEMENTATION_STATUS.md)

## Architecture

The backend follows a layered architecture that connects the Rust core to SwiftUI:

1.  **SwiftUI Layer:** The top layer consists of native SwiftUI views that render the UI.
2.  **Swift Abstraction Layer:** An idiomatic Swift layer that wraps the C FFI, managing the lifecycle of UI components and their state. It translates the reactive concepts from `nami` (`Binding`, `Computed`) into SwiftUI-compatible objects.
3.  **C FFI Layer (`CWaterUI`):** A thin C interface generated from the `waterui-ffi` crate. It defines the stable C ABI for all data structures and functions, allowing communication between Swift and Rust.
4.  **Rust Core (`waterui-core`):** The core logic of the WaterUI framework. It manages the application state, processes events, and provides the view definitions that are sent to the backend.

## Compatibility

This backend is under active development. The current targets are:

*   **iOS:** 26.0 and later
*   **macOS:** 26.0 and later
*   **iPadOS:** 26.0 and later

## Building

To build the project, you can use the Swift Package Manager:

```bash
swift build
```

To use this backend, it must be integrated with the main WaterUI application. Please see the main [WaterUI repository](https://github.com/waterui/waterui) for instructions on building a complete application.

## Contributing

Contributions are welcome! Please feel free to submit a pull request or open an issue if you find a bug or have a feature request.