# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/water-rs/waterui/releases/tag/navigation-v0.2.0) - 2025-12-13

### Added

- Enhance window management and hot reload functionality
- Enhance navigation components with tab positioning and FFI support
- Revamp README and enhance Android hot reload functionality
- Add RichTextEditor component and enhance TextField with line limit functionality
- Introduce StretchAxis for layout management
- Enhance local development mode for WaterUI
- enhance Dockerfile and documentation for improved build and configuration
- *(cli)* enhance Android backend integration with improved logging and automation
- *(event)* introduce event handling system with OnEvent and lifecycle associations
- add waterui-controls module with button, slider, stepper, toggle, and text field components; update dependencies in related Cargo.toml files
- enhance NavigationReceiver with push and pop methods; update raw_view macro to include panic info
- *(android)* Enhance project creation and packaging
- Add Markdown support for rich text rendering and enhance entry point documentation
- Implement Android backend components and runtime
- enhance deployment workflow and add rustdoc integration; update README and introduce roadmap
- update dependencies with versioning and improve documentation clarity
- add counter example to README and enhance tutorial book with tests
- Enhance form components with new derive macros and examples
- *(cli)* add WaterUI CLI for project management

### Fixed

- update documentation to reflect Android View terminology for consistency
- correct spelling errors and improve comments across the codebase
- Refactor reactive system for nami API updates
- *(docs)* resolve doc test compilation errors

### Other

- Bump waterui version to 0.2 in documentation across multiple components
- Bump versions for multiple components to 0.2.0 and introduce waterui-macros crate
- Add waterui-color, waterui-str, and waterui-url crates with comprehensive documentation
- Update README and FFI components for consistency and clarity
- Remove deprecated files and streamline project structure
- Remove AGENT.md and enhance FFI bindings for events and gestures
- Update android backend submodule and enhance clean command functionality
- Remove terminal backend mention from README.md
- Update documentation and add CMake checks for Apple builds
- Make Suspense/hot reload use thread-safe executor
- update FFI header regeneration instructions and enhance README content
- simplify volume binding and toggle handling in video and accordion components
- rename NavigationReceiver to NavigationController for consistency; update related trait and implementation
- update ViewBuilder trait and its implementations for improved usability; simplify view construction across components
- Support multi-platform packaging
- Remove example files for FormBuilder and #[form] macro
- Refactor layout documentation and examples for clarity and completeness
- Refactor and enhance documentation for WaterUI components
- Refactor and enhance components and utilities
- Refactor graphics components and integrate color handling
- Refactor layout components and improve documentation
- Break change in FFI: new vtable-based array API
- update README with enhanced descriptions, quick start guide, and roadmap; improve layout section
- Update README.md
- update README to clarify framework features and demo; remove outdated sections
- Refactor documentation and examples across components for clarity and consistency
- Make compiler happy
- Refactor dependencies in Cargo.toml to use workspace references for waterui-str and nami.
- Completely rewrite README.md with modern WaterUI patterns
- Update README files for clarity and consistency across components
- Refactor WaterUI to integrate nami for reactive state management
- Add more lints and docs
- Add more lints and fix all warnings
- Remove unfinished FFI modules from main branch
- Last version of UniFFI...This crate bring too much complexity for us!
- Add Uniffi scaffolding and FFI support for form components
- Use uniffi for FFI binding
- Polish document
- Refine project and Migrate to Rust 2024
- Redeisgn modifier and add some convenient initializer for components
- Refactor our project
- Reorganize crates, merge main crate and core crate. Better async view support
