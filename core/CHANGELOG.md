# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/water-rs/waterui/compare/core-v0.1.0...core-v0.1.1) - 2025-12-13

### Added

- Enhance window management and hot reload functionality
- Add star field example for WaterUI framework
- Enhance FFI support for metadata and improve Rust bindings
- Introduce permission management for playground projects
- Add CLAUDE.md for project guidance and build instructions
- Implement interruptible command execution and secure metadata handling
- Enhance panic reporting and logging in hot reload system
- Add RichTextEditor component and enhance TextField with line limit functionality
- Introduce StretchAxis for layout management
- Add initial test_example.rs for dynamic binding demonstration
- introduce waterui.h header and enhance theme system with color and font slots
- enhance Dockerfile and documentation for improved build and configuration
- Add watcher functionality for AnyViews and related types
- add TUI platform support and enhance hot reload functionality
- *(event)* introduce event handling system with OnEvent and lifecycle associations
- *(view)* introduce NativeView trait for native implementations and update related components
- enhance NavigationReceiver with push and pop methods; update raw_view macro to include panic info
- Enhance WaterUI with new table and list components
- Implement WuiAnyViewCollection and WuiAnyViews for efficient view management
- Enhance typography and UI components with new font styles and improved view handling
- Introduce WuiFixedContainer for fixed layout management
- Add waterui-color workspace and integrate color resolution
- immigrate to new layout engine for swift backend
- add nightly feature support for conditional compilation and improve raw view handling

### Fixed

- update documentation to reflect Android View terminology for consistency
- correct spelling errors and improve comments across the codebase
- *(core)* use core::ops instead of std::ops for no_std compatibility
- refactor DynamicHandler to manage connection state and improve view updates
- Refactor reactive system for nami API updates
- *(docs)* resolve doc test compilation errors in waterui-core
- *(core)* resolve clippy warning in dynamic component

### Other

- Refactor layout tests to use approximate equality for floating-point comparisons
- Add waterui-color, waterui-str, and waterui-url crates with comprehensive documentation
- streamline media picker and loading state management
- Refactor Native Component and Improve Error Handling
- Enhance documentation and improve code clarity across multiple modules
- Refactor layout components and improve documentation
- Remove deprecated files and streamline project structure
- Update android backend submodule and enhance clean command functionality
- Update components to utilize StretchAxis for layout management
- Refactor code structure for improved readability and maintainability
- Refactor FFI bindings and hot reload functionality
- remove NativeView trait and related implementations; simplify type ID handling in FFI
- update ViewBuilder trait and its implementations for improved usability; simplify view construction across components
- Improve code formatting and readability across multiple files
- Update component imports to use waterui_core and streamline module structure
- Remove example files for FormBuilder and #[form] macro
- replace all .unwrap() to .expect()
- Add workspace configuration and refactor layout imports for consistency
- Refactor layout components to use FixedContainer
- Add accessibility module and enhance view components with accessibility features
- Refactor layout documentation and examples for clarity and completeness
- Refactor and enhance documentation for WaterUI components
- Refactor and enhance components and utilities
- Refactor graphics components and integrate color handling
- Refactor and enhance the WaterUI framework
- update README with enhanced descriptions, quick start guide, and roadmap; improve layout section
- Format & Fix CLI
