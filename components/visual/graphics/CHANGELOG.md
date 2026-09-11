# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0](https://github.com/water-rs/waterui/compare/graphics-v0.3.0...graphics-v0.4.0) - 2026-09-11

### Added

- *(ffi)* capture Android view subtrees into the filter pipeline through AHardwareBuffer
- *(graphics)* let a picture offer its own accessible name
- *(gpu-surface)* give a self-drawn surface an accessibility label
- *(graphics)* draw image brushes in the hybrid scene engine

### Fixed

- *(graphics)* pad image brushes to texel centres on the hybrid engine
- *(graphics)* poll the device after every presented frame
- *(graphics)* keep the wgpu instance for the life of the process
- *(hydrolysis)* make direct_to_target work on HiDPI, one format, opaque only
- *(hydrolysis)* choose the scene engine from the adapter
- *(graphics)* join the GPU completion thread before its runtime drops

### Other

- *(deps)* vello_cpu 0.2 and the remaining tooling bumps
- *(wasm)* lint the wasm32 lane instead of only compiling it
- Merge pull request #392 from water-rs/agent/math-a11y-mathml-273
- Merge pull request #391 from water-rs/agent/hybrid-pad-texel-centre-234
- Merge branch 'dev' into agent/scene-invalidator-watch-274/20260906-024222
- Merge pull request #291 from water-rs/agent/gpu-instance-process-lifetime
- Merge pull request #278 from water-rs/agent/direct-to-target-hidpi/20260902-140021
- *(graphics)* lay the image-brush snapshots out as suite/case/stage
- *(graphics)* assert the image brush against its source, not the other engine
- Merge pull request #224 from water-rs/agent/graphics-recursion-limit

## [0.3.0](https://github.com/water-rs/waterui/compare/graphics-v0.2.1...graphics-v0.3.0) - 2026-08-25

### Added

- Added GPU-first scene composition, shared shader artifacts, color-management support, and portable renderer integration.

## [0.2.1](https://github.com/water-rs/waterui/compare/graphics-v0.2.0...graphics-v0.2.1) - 2025-12-14

### Fixed

- update README and Cargo.toml files to specify README.md for all components

## [0.2.0](https://github.com/water-rs/waterui/releases/tag/graphics-v0.2.0) - 2025-12-13

### Added

- implement canvas module with path, state, gradient, image, and text functionalities
- Enhance window management and hot reload functionality
- Add mac screenshot image and improve markdown example
- Add star field example for WaterUI framework
- enhance logging levels for Apple platforms and improve app structure
- Introduce Canvas module for 2D vector graphics rendering
- Add ShaderSurface for simplified GPU rendering
- Introduce GpuSurface for high-performance GPU rendering
- Revamp README and enhance Android hot reload functionality
- Enhance Android packaging and hot reload functionality
- Introduce StretchAxis for layout management
- Enhance local development mode for WaterUI
- enhance Dockerfile and documentation for improved build and configuration
- _(cli)_ enhance Android backend integration with improved logging and automation
- Enhance GPU surface handling and shader rendering in graphics module
- _(graphics)_ add renderer view support and CPU rendering capabilities
- _(android)_ Enhance project creation and packaging
- Add Markdown support for rich text rendering and enhance entry point documentation
- Implement Android backend components and runtime
- Enhance graphics context with Debug implementation and add alignment methods for layout components
- Add waterui-color workspace and integrate color resolution
- enhance deployment workflow and add rustdoc integration; update README and introduce roadmap
- update dependencies with versioning and improve documentation clarity
- add counter example to README and enhance tutorial book with tests
- _(cli)_ add WaterUI CLI for project management

### Fixed

- remove unused 'parley' dependency from canvas feature in Cargo.toml
- improve code formatting and readability across multiple files
- update documentation to reflect Android View terminology for consistency
- correct spelling errors and improve comments across the codebase
- _(docs)_ resolve doc test compilation errors

### Other

- Bump waterui version to 0.2 in documentation across multiple components
- Bump versions for multiple components to 0.2.0 and introduce waterui-macros crate
- Disable canvas on main branch, since crates.io release do not allow git reference
- remove WASM job from CI workflow and unused parley dependency
- Switch color APIs to use ResolvedColor
- improve code readability and consistency across multiple files
- Merge feature/full-featured-canvas into dev
- Add waterui-color, waterui-str, and waterui-url crates with comprehensive documentation
- Refactor Native Component and Improve Error Handling
- Enhance documentation and improve code clarity across multiple modules
- Refactor and clean up code across multiple components
- Refactor layout components and improve documentation
- Update README and FFI components for consistency and clarity
- Streamline playground project handling and enhance build process
- Remove AGENT.md and enhance FFI bindings for events and gestures
- Update android backend submodule and enhance clean command functionality
- Remove terminal backend mention from README.md
- Update documentation and add CMake checks for Apple builds
- Make Suspense/hot reload use thread-safe executor
- update FFI header regeneration instructions and enhance README content
- Support multi-platform packaging
- Update dependencies and improve code structure across multiple modules
- Remove example files for FormBuilder and #[form] macro
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
- Refactor documentation and examples across components for clarity and consistency
- Refactor dependencies in Cargo.toml to use workspace references for waterui-str and nami.
- Completely rewrite README.md with modern WaterUI patterns
- Update README files for clarity and consistency across components
- Add more lints and fix all warnings
- Refine project and Migrate to Rust 2024
- Redesign modifier and add some convenient initializer for components
- Refactor our project
- Reorganize crates, merge main crate and core crate. Better async view support
