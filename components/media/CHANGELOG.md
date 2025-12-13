# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/water-rs/waterui/releases/tag/media-v0.2.0) - 2025-12-13

### Added

- Enhance window management and hot reload functionality
- _(media-picker)_ add media picker example with photo, video, and live photo selection
- Add mac screenshot image and improve markdown example
- _(android)_ add support for multiple architectures and clean jniLibs
- Update dependencies and enhance backend configurations
- Update media components with enhanced MediaPicker support and new features
- Refactor Photo component and enhance hot reload functionality
- Revamp README and enhance Android hot reload functionality
- Enhance video component functionality and interrupt handling
- Enhance panic reporting and logging in hot reload system
- Introduce StretchAxis for layout management
- Enhance local development mode for WaterUI
- enhance Dockerfile and documentation for improved build and configuration
- _(cli)_ enhance Android backend integration with improved logging and automation
- _(android)_ Enhance project creation and packaging
- Add Markdown support for rich text rendering and enhance entry point documentation
- Implement Android backend components and runtime
- Add image handling capabilities with new Image struct and integrate into Photo component
- enhance deployment workflow and add rustdoc integration; update README and introduce roadmap
- update dependencies with versioning and improve documentation clarity
- add counter example to README and enhance tutorial book with tests
- Enhance form components with new derive macros and examples
- _(ffi)_ enhance FFI bindings with new types and conversions for media and form components
- _(cli)_ add WaterUI CLI for project management

### Fixed

- update documentation to reflect Android View terminology for consistency
- correct spelling errors and improve comments across the codebase
- Refactor reactive system for nami API updates
- _(docs)_ resolve doc test compilation errors
- Update code block syntax in documentation for clarity
- Make FFI around waterui-media compiled

### Other

- Bump waterui version to 0.2 in documentation across multiple components
- Bump versions for multiple components to 0.2.0 and introduce waterui-macros crate
- Refactor layout tests to use approximate equality for floating-point comparisons
- _(media-picker)_ enhance media selection handling and error management
- Add waterui-color, waterui-str, and waterui-url crates with comprehensive documentation
- streamline media picker and loading state management
- Refactor Native Component and Improve Error Handling
- Enhance documentation and improve code clarity across multiple modules
- Refactor and clean up code across multiple components
- Refactor layout components and improve documentation
- Update README and FFI components for consistency and clarity
- Clean up project structure and enhance CLI functionality
- Remove AGENT.md and enhance FFI bindings for events and gestures
- Update components to utilize StretchAxis for layout management
- Remove terminal backend mention from README.md
- Update documentation and add CMake checks for Apple builds
- Make Suspense/hot reload use thread-safe executor
- update FFI header regeneration instructions and enhance README content
- simplify volume binding and toggle handling in video and accordion components
- Support multi-platform packaging
- Improve code formatting and readability across multiple files
- Update component imports to use waterui_core and streamline module structure
- Remove example files for FormBuilder and #[form] macro
- replace all .unwrap() to .expect()
- Refactor layout documentation and examples for clarity and completeness
- Refactor and enhance documentation for WaterUI components
- Use f32 for any layout relate variable (which is more efficiency)
- Refactor and enhance components and utilities
- Refactor layout components and improve documentation
- Break change in FFI: new vtable-based array API
- update README with enhanced descriptions, quick start guide, and roadmap; improve layout section
- Update README.md
- update README to clarify framework features and demo; remove outdated sections
- Refactor documentation and examples across components for clarity and consistency
- Make compiler happy
- Update Cargo.toml members and improve documentation in various modules
- Refactor dependencies in Cargo.toml to use workspace references for waterui-str and nami.
- Add URL utility crate & cleanup the book structure
- Remove unused SecureField configuration and add Url struct with conversion implementations
- Completely rewrite README.md with modern WaterUI patterns
- Update README files for clarity and consistency across components
- Refactor WaterUI to integrate nami for reactive state management
- Add more lints and docs
- Add more lints and fix all warnings
- Remove unfinished FFI modules from main branch
- New reactive API
- Add Uniffi scaffolding and FFI support for form components
- Use uniffi for FFI binding
- Refine project and Migrate to Rust 2024
- Redesign modifier and add some convenient initializer for components
- Refactor our project
- Reorganize crates, merge main crate and core crate. Better async view support
