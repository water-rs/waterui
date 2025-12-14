# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/water-rs/waterui/compare/controls-v0.2.0...controls-v0.2.1) - 2025-12-14

### Fixed

- update README and Cargo.toml files to specify README.md for all components

## [0.2.0](https://github.com/water-rs/waterui/releases/tag/controls-v0.2.0) - 2025-12-13

### Added

- Enhance window management and hot reload functionality
- Add star field example for WaterUI framework
- Revamp README and enhance Android hot reload functionality
- Implement interruptible command execution and secure metadata handling
- Add RichTextEditor component and enhance TextField with line limit functionality
- Introduce StretchAxis for layout management
- Enhance local development mode for WaterUI
- enhance Dockerfile and documentation for improved build and configuration
- _(cli)_ enhance Android backend integration with improved logging and automation
- add waterui-controls module with button, slider, stepper, toggle, and text field components; update dependencies in related Cargo.toml files
- _(android)_ Enhance project creation and packaging
- Add Markdown support for rich text rendering and enhance entry point documentation
- Implement Android backend components and runtime
- enhance deployment workflow and add rustdoc integration; update README and introduce roadmap
- update dependencies with versioning and improve documentation clarity
- add counter example to README and enhance tutorial book with tests
- _(cli)_ add WaterUI CLI for project management

### Fixed

- update documentation to reflect Android View terminology for consistency
- correct spelling errors and improve comments across the codebase
- _(docs)_ resolve doc test compilation errors

### Other

- Bump waterui version to 0.2 in documentation across multiple components
- Bump versions for multiple components to 0.2.0 and introduce waterui-macros crate
- update Cargo.toml files to improve workspace structure and add dev-dependencies
- Remove canvas example and update dependencies in other examples
- improve code readability and consistency across multiple files
- Refactor layout tests to use approximate equality for floating-point comparisons
- Add waterui-color, waterui-str, and waterui-url crates with comprehensive documentation
- Enhance documentation and improve code clarity across multiple modules
- Refactor layout components and improve documentation
- Update README and FFI components for consistency and clarity
- Remove AGENT.md and enhance FFI bindings for events and gestures
- Standardize library naming for Android and Apple backends
- Add illustrations for various components including Button, Slider, Stepper, TextField, Toggle, Grid, Stack, and Text
- button illustration
- Update components to utilize StretchAxis for layout management
- Remove obsolete layout and issue report files
- Remove terminal backend mention from README.md
- Update documentation and add CMake checks for Apple builds
- Make Suspense/hot reload use thread-safe executor
- update FFI header regeneration instructions and enhance README content
- Support multi-platform packaging
- Remove example files for FormBuilder and #[form] macro
- Refactor layout documentation and examples for clarity and completeness
- Refactor and enhance documentation for WaterUI components
- Refactor and enhance components and utilities
- Refactor layout components and improve documentation
- Break change in FFI: new vtable-based array API
- update README with enhanced descriptions, quick start guide, and roadmap; improve layout section
- Update README.md
- update README to clarify framework features and demo; remove outdated sections
- Refactor documentation and examples across components for clarity and consistency
- Refactor dependencies in Cargo.toml to use workspace references for waterui-str and nami.
- Completely rewrite README.md with modern WaterUI patterns
- Update README files for clarity and consistency across components
- Add more lints and fix all warnings
- Refine project and Migrate to Rust 2024
- Redesign modifier and add some convenient initializer for components
- Refactor our project
- Reorganize crates, merge main crate and core crate. Better async view support
