# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/water-rs/waterui/compare/form-v0.2.0...form-v0.2.1) - 2025-12-14

### Fixed

- update README and Cargo.toml files to specify README.md for all components

## [0.2.0](https://github.com/water-rs/waterui/releases/tag/form-v0.2.0) - 2025-12-13

### Added

- Enhance window management and hot reload functionality
- Revamp README and enhance Android hot reload functionality
- Implement interruptible command execution and secure metadata handling
- Introduce StretchAxis for layout management
- Enhance local development mode for WaterUI
- enhance Dockerfile and documentation for improved build and configuration
- _(cli)_ enhance Android backend integration with improved logging and automation
- add validation utilities for form components; implement Validatable and Validator traits
- add waterui-controls module with button, slider, stepper, toggle, and text field components; update dependencies in related Cargo.toml files
- _(android)_ Enhance project creation and packaging
- Add Markdown support for rich text rendering and enhance entry point documentation
- Implement Android backend components and runtime
- enhance deployment workflow and add rustdoc integration; update README and introduce roadmap
- update dependencies with versioning and improve documentation clarity
- add counter example to README and enhance tutorial book with tests
- implement ComputedFont class and enhance Text component; update ScrollView initialization and demo structure
- Enhance form components with new derive macros and examples
- _(cli)_ add WaterUI CLI for project management
- Implement custom layout engine integration with GTK4
- Add GTK4 backend for WaterUI framework

### Fixed

- update dependencies and improve path handling in template context
- update documentation to reflect Android View terminology for consistency
- correct spelling errors and improve comments across the codebase
- Refactor reactive system for nami API updates
- _(docs)_ resolve doc test compilation errors
- Update code block syntax in documentation for clarity
- correct import path for FormBuilder in derive_test example

### Other

- Bump waterui version to 0.2 in documentation across multiple components
- Bump versions for multiple components to 0.2.0 and introduce waterui-macros crate
- update Cargo.toml files to improve workspace structure and add dev-dependencies
- Remove canvas example and update dependencies in other examples
- remove outdated comments from Validator trait in valid.rs
- improve code readability and consistency across multiple files
- Refactor layout tests to use approximate equality for floating-point comparisons
- Add waterui-color, waterui-str, and waterui-url crates with comprehensive documentation
- Add playground backend paths and target dir support
- Enhance documentation and improve code clarity across multiple modules
- Refactor and clean up code across multiple components
- Refactor layout components and improve documentation
- Update README and FFI components for consistency and clarity
- Update Apple backend submodule and add video player example
- Remove AGENT.md and enhance FFI bindings for events and gestures
- Update components to utilize StretchAxis for layout management
- Remove obsolete layout and issue report files
- Remove terminal backend mention from README.md
- Update documentation and add CMake checks for Apple builds
- Make Suspense/hot reload use thread-safe executor
- update FFI header regeneration instructions and enhance README content
- Align Android tooling and agent workflow
- Support multi-platform packaging
- Update dependencies and improve code structure across multiple modules
- Remove example files for FormBuilder and #[form] macro
- replace all .unwrap() to .expect()
- Refactor layout components to use FixedContainer
- Refactor layout documentation and examples for clarity and completeness
- Refactor and enhance documentation for WaterUI components
- Use f32 for any layout relate variable (which is more efficiency)
- Refactor and enhance components and utilities
- Refactor graphics components and integrate color handling
- Refactor layout components and improve documentation
- Break change in FFI: new vtable-based array API
- update README with enhanced descriptions, quick start guide, and roadmap; improve layout section
- Add MIT license
- Update README.md
- update README to clarify framework features and demo; remove outdated sections
- Refactor documentation and examples across components for clarity and consistency
- enhance FormBuilder derive macro and examples; update form handling and component mappings
- Make compiler happy
- Refactor dependencies in Cargo.toml to use workspace references for waterui-str and nami.
- Remove unused SecureField configuration and add Url struct with conversion implementations
- Completely rewrite README.md with modern WaterUI patterns
- Add tutorial book and window management module
- Update README files for clarity and consistency across components
- Refactor WaterUI to integrate nami for reactive state management
- Add more lints and docs
- Add more lints and fix all warnings
- Remove unfinished FFI modules from main branch
- Add Uniffi scaffolding and FFI support for form components
- Polish document
- Refine project and Migrate to Rust 2024
- Redesign modifier and add some convenient initializer for components
- Refactor our project
- Reorganize crates, merge main crate and core crate. Better async view support
