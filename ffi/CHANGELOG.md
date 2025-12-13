# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/water-rs/waterui/releases/tag/ffi-v0.2.0) - 2025-12-13

### Added

- Enhance window management and hot reload functionality
- _(media-picker)_ add media picker example with photo, video, and live photo selection
- Add mac screenshot image and improve markdown example
- Add star field example for WaterUI framework
- Enhance panic logging and error handling in Apple backends
- _(flame)_ enhance flame shader for HDR rendering and adjust GPU surface format
- enhance logging levels for Apple platforms and improve app structure
- _(android)_ streamline APK installation process and add CMake toolchain wrapper for ABI support
- Rewrite CLI
- Implement hot reload server and connection handling
- Update media components with enhanced MediaPicker support and new features
- Refactor Photo component and enhance hot reload functionality
- Refactor toolchain error handling and enhance hot reload configuration
- Introduce GpuSurface for high-performance GPU rendering
- Enhance navigation components with tab positioning and FFI support
- Revamp README and enhance Android hot reload functionality
- Enhance video component functionality and interrupt handling
- Enhance FFI support for metadata and improve Rust bindings
- Add CLAUDE.md for project guidance and build instructions
- Implement interruptible command execution and secure metadata handling
- Enhance panic reporting and logging in hot reload system
- Introduce StretchAxis for layout management
- Introduce comprehensive logging and panic reporting plan
- Enhance debugging and layout system in Apple backend
- Add layout issue report for axis-expanding views on Apple backend
- Implement safe area handling in layout system
- Enhance local development mode for WaterUI
- Add initial test_example.rs for dynamic binding demonstration
- introduce waterui.h header and enhance theme system with color and font slots
- enhance Dockerfile and documentation for improved build and configuration
- _(ci)_ add new workflows for CLI and Docker builds, enhance CI checks, and implement changelog generation
- _(logging)_ enhance tracing and panic forwarding for improved log management
- _(hot_reload)_ implement hot reload support for FFI and backend configurations
- update dependencies to use git sources and improve error handling in CLI
- _(theme)_ add background and surface color functions to FFI and include tests for readability
- _(theme)_ add macro definitions for theme color and font functions
- Add watcher functionality for AnyViews and related types
- _(theme)_ add theme module with color and font functions for FFI
- document hot reload support for Android, Apple, Web, and TUI backends
- add TUI platform support and enhance hot reload functionality
- Introduce build command for native artifacts
- _(android)_ enhance panic hook with additional string utilities
- add miette integration for enhanced error handling and logging
- _(cli)_ enhance Android backend integration with improved logging and automation
- enhance Android backend integration and logging capabilities
- Add new dependencies for enhanced functionality and refactor hot reload implementation
- Implement hot reload functionality with configurable environment
- enhance CI workflows and update C header documentation
- add pre-commit configuration for Rust formatting with rustfmt
- Implement JNI bindings for Android platform in FFI layer
- Enhance Android and watchOS support with new SDK checks and build scripts
- Enhance WaterUI with new table and list components
- Implement WuiAnyViewCollection and WuiAnyViews for efficient view management
- _(graphics)_ add renderer view support and CPU rendering capabilities
- _(android)_ Enhance project creation and packaging
- Add OKLCH color space support and enhance color module documentation
- Add Markdown support for rich text rendering and enhance entry point documentation
- Implement gesture handling in WaterUI with WuiGestureView and GestureSequenceContainer
- Implement gesture handling with FFI support and define gesture event payloads
- Implement WuiList and WuiTable components with FFI support and enhance tree and accordion structures
- Introduce WuiFixedContainer for fixed layout management
- Implement Android backend components and runtime
- Enhance graphics context with Debug implementation and add alignment methods for layout components
- Add waterui-color workspace and integrate color resolution
- immigrate to new layout engine for swift backend
- enhance deployment workflow and add rustdoc integration; update README and introduce roadmap
- update dependencies with versioning and improve documentation clarity
- add counter example to README and enhance tutorial book with tests
- implement ComputedFont class and enhance Text component; update ScrollView initialization and demo structure
- add Divider, Scroll, and Spacer components; implement WuiProgress in FFI
- Hello,world! Welcome to WaterUI world!!! Now demo works!
- Initialize demo project with WaterUI integration
- Add template member to workspace and enhance FFI with new environment handling
- Enhance form components with new derive macros and examples
- _(ffi)_ enhance FFI bindings with new types and conversions for media and form components
- _(cli)_ add WaterUI CLI for project management

### Fixed

- improve code formatting and readability across multiple files
- update documentation to reflect Android View terminology for consistency
- correct spelling errors and improve comments across the codebase
- Update FFI function signature
- _(ffi)_ import necessary traits for hot reload directory configuration
- _(hot_reload)_ import necessary traits for hot reload endpoint configuration
- _(ffi)_ ensure thread safety in waterui_init and waterui_main functions
- rename metadata functions for consistency and clarity
- correct function naming for FFI compatibility in macros
- Refactor reactive system for nami API updates
- _(docs)_ resolve doc test compilation errors
- Run formatter
- Make FFI around waterui-media compiled

### Other

- Bump waterui version to 0.2 in documentation across multiple components
- Bump versions for multiple components to 0.2.0 and introduce waterui-macros crate
- Switch color APIs to use ResolvedColor
- Refactor layout tests to use approximate equality for floating-point comparisons
- Add waterui-color, waterui-str, and waterui-url crates with comprehensive documentation
- streamline media picker and loading state management
- Refactor Native Component and Improve Error Handling
- Remove TUI backend and associated files
- Enhance Android and Apple backends with permission management and Gradle build improvements
- Add AndroidEmulator device and defer launch to CLI
- Switch zenwave/skyzen to crates.io versions
- Enhance documentation and improve code clarity across multiple modules
- Refactor and clean up code across multiple components
- Refactor layout components and improve documentation
- Update Cargo.lock and FFI components for hot reload configuration
- Update README and FFI components for consistency and clarity
- Improve URL parsing and validation logic
- Update FFI header generation and enhance tab positioning documentation
- Update hot reload configuration and enhance dependencies
- Update Android backend submodule and enhance FFI function signature
- Remove deprecated files and streamline project structure
- Clean up project structure and enhance CLI functionality
- Remove AGENT.md and enhance FFI bindings for events and gestures
- Standardize library naming for Android and Apple backends
- Update android backend submodule and enhance clean command functionality
- Update components to utilize StretchAxis for layout management
- Remove terminal backend mention from README.md
- Update submodule references and enhance FFI bindings
- Update documentation and add CMake checks for Apple builds
- Update submodule references for android and apple backends
- Enhance GEMINI_ASSISTANT documentation with debugging and research best practices
- Fix ffi_computed watch function to accept WuiComputed<T> instead of Computed<T>
- update submodule branches and clean up unused files
- Make Suspense/hot reload use thread-safe executor
- update FFI header regeneration instructions and enhance README content
- simplify panic logging and improve message formatting
- remove Android-specific module and clean up Cargo.toml
- Refactor code structure for improved readability and maintainability
- Refactor FFI bindings and hot reload functionality
- remove NativeView trait and related implementations; simplify type ID handling in FFI
- rename NavigationReceiver to NavigationController for consistency; update related trait and implementation
- update ViewBuilder trait and its implementations for improved usability; simplify view construction across components
- reorganize imports in form and navigation components for clarity
- Support multi-platform packaging
- Update dependencies and improve code structure across multiple modules
- Improve code formatting and readability across multiple files
- Update component imports to use waterui_core and streamline module structure
- Remove example files for FormBuilder and #[form] macro
- replace all .unwrap() to .expect()
- Use `Views` trait in table, list and lazy view
- Add workspace configuration and refactor layout imports for consistency
- Refactor layout components to use FixedContainer
- Fix swift backend
- Refactor WaterUI: Update StyledStr Handling and Remove Unused Components
- Refactor layout documentation and examples for clarity and completeness
- Refactor and enhance documentation for WaterUI components
- Use f32 for any layout relate variable (which is more efficiency)
- Refactor and enhance components and utilities
- Refactor graphics components and integrate color handling
- Refactor and enhance the WaterUI framework
- Refactor layout components and improve documentation
- Break change in FFI: new vtable-based array API
- update README with enhanced descriptions, quick start guide, and roadmap; improve layout section
- Update README.md
- update README to clarify framework features and demo; remove outdated sections
- improve code formatting and readability across multiple files; enhance memory management in Rust FFI
- Refactor documentation and examples across components for clarity and consistency
- Implement FFI bindings for navigation and text components, enhancing the interoperability of the WaterUI framework with C. This includes the addition of structures for navigation views, links, and tabs, as well as text configurations and font representations. The changes also streamline the conversion between Rust and FFI types, ensuring safe memory management and improved performance. Additionally, several utility functions for string manipulation have been updated to return pointers instead of structures, optimizing memory usage and access patterns.
- Refactor tutorial content for WaterUI
- Add FFI bindings for WaterUI and enhance Str utility functions
- Update Cargo.toml members and improve documentation in various modules
- Refactor dependencies in Cargo.toml to use workspace references for waterui-str and nami.
- Bring back `waterui_ffi`
- Completely rewrite README.md with modern WaterUI patterns
- Update README files for clarity and consistency across components
- Add more lints and fix all warnings
- Use uniffi for FFI binding
- Last version support C-API. May it is too early to consider this. Currently let's just focus on the interaction between renderer. And performance consideration could be delayed until we meet the bottleneck.
- Refine project and Migrate to Rust 2024
- Redesign modifier and add some convenient initializer for components
- Refactor our project
- Reorganize crates, merge main crate and core crate. Better async view support
