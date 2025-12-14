# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.2](https://github.com/water-rs/waterui/compare/cli-v0.1.1...cli-v0.1.2) - 2025-12-14

### Fixed

- update android backend dependency version to 0.2.0
- update minimum version for Swift package reference to 0.2.0
- update repository URLs in template files for Apple and Android backends

## [0.1.0](https://github.com/water-rs/waterui/releases/tag/cli-v0.1.0) - 2025-12-13

### Added

- Enhance window management and hot reload functionality
- _(media-picker)_ add media picker example with photo, video, and live photo selection
- Add mac screenshot image and improve markdown example
- _(cli)_ enable hot reload by default for water run
- Enhance panic logging and error handling in Apple backends
- enhance logging levels for Apple platforms and improve app structure
- _(examples)_ add gesture and list examples with corresponding templates and assets
- _(android)_ streamline APK installation process and add CMake toolchain wrapper for ABI support
- _(android)_ switch from ComponentActivity to AppCompatActivity and disable Compose
- _(android)_ implement multi-ABI packaging support and optimize build configurations
- _(android)_ add support for multiple architectures and clean jniLibs
- Rewrite CLI
- Add project creation with git initialization and new playground example
- Update dependencies and enhance backend configurations
- Implement hot reload server and connection handling
- Enhance Apple platform support with new platform kinds and SDKs
- Add initial Android and Apple project templates
- enhance Android and Apple platform support with new configurations and utility functions
- update platform implementations for Android and Apple, improve function signatures and add architecture handling
- enhance platform support and add new build options for Android and Apple
- enhance Android and Apple device support with new functionalities
- enhance Android and Apple platform support, add Homebrew toolchain manager
- _(android)_ refactor backend and platform modules
- Enhance async support and device scanning in backend implementations
- Refactor toolchain error handling and enhance hot reload configuration
- Implement report generation for various command results
- Introduce async runtime and enhance file watcher functionality
- Revamp README and enhance Android hot reload functionality
- Enhance video component functionality and interrupt handling
- Enhance Android packaging and hot reload functionality
- Introduce permission management for playground projects
- Add CLAUDE.md for project guidance and build instructions
- Implement interruptible command execution and secure metadata handling
- Enhance panic reporting and logging in hot reload system
- Introduce comprehensive logging and panic reporting plan
- Enhance debugging and layout system in Apple backend
- Implement safe area handling in layout system
- Enhance local development mode for WaterUI
- Rename Android library to libwaterui_app.so
- add Gemini CLI assistant documentation and subagent manager script
- add Git commit hash to build output and update dependencies
- enhance Dockerfile and documentation for improved build and configuration
- enhance Android platform support with target triples and improve artifact stripping
- integrate hyper and tungstenite for hot reload server functionality
- _(run)_ implement hot reload support and ensure cdylib generation
- _(android)_ enhance NDK toolchain checks and environment configuration
- _(logging)_ add log filter support for hot reload and CLI
- _(logging)_ enhance tracing and panic forwarding for improved log management
- _(hot_reload)_ implement hot reload support for FFI and backend configurations
- update dependencies to use git sources and improve error handling in CLI
- _(android)_ enhance Java environment detection for macOS by including Android Studio's bundled JBR
- add debugging workflow checklist and enhance non-interactive terminal handling
- Add watcher functionality for AnyViews and related types
- _(apple)_ enhance simulator boot logic and add state checking
- _(device)_ add platform filtering for device listing
- document hot reload support for Android, Apple, Web, and TUI backends
- add TUI platform support and enhance hot reload functionality
- _(hot-reload)_ enhance hot reload configuration and update related commands
- Introduce build command for native artifacts
- _(cli)_ enhance Android backend integration with improved logging and automation
- enhance Android backend integration and logging capabilities
- _(cli)_ add backend list command
- _(cli)_ support backend upgrade with ffi checks
- _(cli)_ add backend management command
- _(cli)_ use local android backend in dev
- Update Android project structure and dependencies for improved backend integration
- Implement hot reload functionality with configurable environment
- _(cli)_ enhance JSON output support across commands
- update build process to emit stable version tags for waterui and swift backend; enhance dependency resolution with branch support
- rename swift backend to apple backend and update related configurations
- update release workflow to remove swift and android backend support; add dynamic repo URL for Swift backend
- Add compiler options to suppress Kotlin version compatibility check for Compose (now app can be compiled but still not launched)
- Update Compose compiler extension version to 1.5.14 in build.gradle.kts and template
- Add Jetpack Compose support and update MainActivity for Compose integration
- Enhance Android project setup by adding sanitized crate name and copying libc++\_shared.so
- Add Android target configurations and enhance toolchain setup for Rust builds
- Enhance Android and watchOS support with new SDK checks and build scripts
- Add Android tool management and build functionality; refactor device handling
- Migrate from anyhow to color_eyre for error handling and add build script for version management
- _(android)_ Enhance project creation and packaging
- Add Markdown support for rich text rendering and enhance entry point documentation
- Add web backend support with asset creation and configuration
- Implement gesture handling in WaterUI with WuiGestureView and GestureSequenceContainer
- Add sccache and mold configuration for improved build performance
- Enhance typography and UI components with new font styles and improved view handling
- Add Xcode project management functions and improve macOS support in run module
- Implement cleanup command to remove build artifacts and platform caches
- Add fix option to WaterUI Doctor for automatic issue resolution
- Implement WaterUI Doctor for environment checks and add new dependencies
- Introduce WuiFixedContainer for fixed layout management
- Add support for multiple backends (Swift and Android) in project configuration and creation
- Implement Android backend components and runtime
- Add waterui-color workspace and integrate color resolution
- add project creation functionality with templates for Android and SwiftUI and remove gtk4 backend
- add CLI package with initial command structure and dependencies
- enhance deployment workflow and add rustdoc integration; update README and introduce roadmap
- update dependencies with versioning and improve documentation clarity
- add counter example to README and enhance tutorial book with tests
- _(cli)_ add WaterUI CLI for project management

### Fixed

- format function parameters for better readability in tests
- update dependencies and improve path handling in template context
- improve code formatting and readability across multiple files
- update documentation to reflect Android View terminology for consistency
- correct spelling errors and improve comments across the codebase
- update llvm-strip command to preserve dynamic symbol table
- improve interrupt handling in wait_for_interrupt function
- remove hot reload import for non-WASM targets
- clean up imports and improve command output messages in Android and Apple device modules
- _(android)_ add INTERNET permission to AndroidManifest for network access
- _(dependencies)_ replace log with tracing for improved logging consistency
- update run functions to handle no_watch parameter for improved behavior
- _(docs)_ resolve doc test compilation errors
- Update RendererViewComponent to use rawPtr for memory management consistency

### Other

- Bump waterui version to 0.2 in documentation across multiple components
- improve code readability and consistency across multiple files
- Refactor layout tests to use approximate equality for floating-point comparisons
- Add waterui-color, waterui-str, and waterui-url crates with comprehensive documentation
- streamline media picker and loading state management
- Refactor Native Component and Improve Error Handling
- Refactor Android device run logic and update Gradle configurations for improved build process
- Add log level management for device log streaming across platforms
- Enhance Android and Apple backends with permission management and Gradle build improvements
- Add AndroidEmulator device and defer launch to CLI
- Embed CLI commit hash and add water dir management
- Add Android SDK path helpers and use adb_path
- Switch zenwave/skyzen to crates.io versions
- Add playground backend paths and target dir support
- Update submodules and enhance macOS launcher UI
- Implement CLI commands for WaterUI: build, clean, create, devices, doctor, package, and run
- Enhance documentation and improve code clarity across multiple modules
- Refactor backend trait and implement new build system
- WTF...Let's rewrite it!
- clean up and enhance project structure and backend definitions
- Refactor CLI toolchain and installation modules
- Refactor and clean up code across multiple components
- Refactor layout components and improve documentation
- Simplify WebSocket close handling and clean up code formatting
- Update workspace configuration and dependencies for examples
- Update README and FFI components for consistency and clarity
- Clean up hot reload session logging and initialization
- Simplify hot reload configuration for Android and Apple devices
- Update hot reload configuration and enhance dependencies
- Remove deprecated files and streamline project structure
- Streamline playground project handling and enhance build process
- Clean up project structure and enhance CLI functionality
- Update backend submodules and enhance CLI device commands
- Refactor CLI commands for device management and enhance roadmap documentation
- Remove AGENT.md and enhance FFI bindings for events and gestures
- Update Android backend submodule and adjust padding in template
- Update Android backend submodule and modify build process
- Standardize library naming for Android and Apple backends
- Update android backend submodule and enhance clean command functionality
- Remove terminal backend mention from README.md
- Update documentation and add CMake checks for Apple builds
- Enhance WaterUI CLI documentation and add screenshot capture feature
- Fix CLI to copy libc++\_shared.so to jniLibs for Android builds
- remove unused hyper and tungstenite dependencies, integrate skyzen for hot reload functionality
- simplify hot reload library path handling and extract filename logic
- Make Suspense/hot reload use thread-safe executor
- prefer Android Studio JBR when Java missing
- update FFI header regeneration instructions and enhance README content
- Align Android tooling and agent workflow
- Refactor Apple backend integration and remove deprecated files
- _(build)_ streamline argument handling in build script
- Enhance UI output handling and add terminal utilities for improved user experience
- Improve command construction and error handling in project build and run functions
- Refactor CLI structure and remove unused modules
- Start to refractor WaterUI CLI...
- Rename no_watch argument to no_hot_reload for clarity and update related function signatures
- Refactor FFI bindings and hot reload functionality
- _(cli)_ enhance cargo build process with sccache configuration and retry logic
- Merge pull request #23 from water-rs/codex/refactor-waterui-cli-for-enhanced-functionality
- Refactor toolchain checks and interactive doctor
- Merge branch 'main' into codex/refactor-cli-to-use-third-party-crates
- Improve CLI web dev server stability
- Merge branch 'main' into codex/add-json-output-support-for-cli
- Add JSON output mode to CLI
- Update Package.swift for waterui and adjust Swift version in backend package
- Update component imports to use waterui_core and streamline module structure
- Remove example files for FormBuilder and #[form] macro
- remove unused development team parameter and clean up code
- Update defaultValue and deinit annotations in Gesture.swift; fix string formatting in project.pbxproj.tpl
- replace all .unwrap() to .expect()
- Refactor CLI Doctor Command and Improve Error Handling
- Removed unusable backends
- Refactor layout documentation and examples for clarity and completeness
- Refactor and enhance documentation for WaterUI components
- Refactor and enhance components and utilities
- Refactor and enhance the WaterUI framework
- Refactor layout components and improve documentation
- Break change in FFI: new vtable-based array API
- update README with enhanced descriptions, quick start guide, and roadmap; improve layout section
- Update README.md
- update README to clarify framework features and demo; remove outdated sections
- Refactor documentation and examples across components for clarity and consistency
- remove CLI module and related files to streamline project structure
- Refactor dependencies in Cargo.toml to use workspace references for waterui-str and nami.
- Clean up whitespace and formatting in various files
- Bring back `waterui_ffi`
- Completely rewrite README.md with modern WaterUI patterns
- Update README files for clarity and consistency across components
- Add more lints and fix all warnings
- Refine project and Migrate to Rust 2024
- Redesign modifier and add some convenient initializer for components
- Refactor our project
- Reorganize crates, merge main crate and core crate. Better async view support
