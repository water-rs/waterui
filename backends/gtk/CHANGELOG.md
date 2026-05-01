# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/water-rs/waterui/releases/tag/waterui-gtk-v0.1.0) - 2026-05-01

### Added

- complete navigation parity across backends
- *(video)* continue player parity work
- *(particle)* add gpu collision physics
- align focus state across backends
- *(graphics)* finalize advanced filter pipeline and core integration
- *(hydrolysis)* wire reactive rebuild loop through ViewBuilder roots
- complete phase 0b and phase 1+2r foundation
- extract opacity from GPU filter pipeline + zero-alloc ViewDispatcher
- continue deep review fixes and ffi fast-fail cleanup
- integrate assets, media, and runtime updates
- introduce GPU texture filtering utility and image example, refactor media components
- *(color)* enhance color handling with HDR support and new FFI functions
- *(color)* enhance color handling with headroom support in conversions
- *(README)* update header layout and add badges for improved visibility and branding
- *(README)* add WaterUI logo and update header for improved branding
- enhance navigation controller with FFI support and renderer integration
- add GTK4 List, Picker, Photo, and SecureField components; update Cargo.toml dependencies
- enhance GPU support in GTK4 backend with wgpu-hal and glow dependencies
- add GTK4 Color and LazyContainer components, and update dependencies for GPU support
- update GTK components to improve layout handling and add padding support
- add GTK backend support to WaterUI CLI
- add GPU surface support with wgpu integration for GTK4 backend
- Implement GTK4 components for WaterUI
- enhance GTK components and layout integration for WaterUI
- add GTK4 backend for WaterUI with core infrastructure and initial components
- Enhance window management and hot reload functionality
- Revamp README and enhance Android hot reload functionality
- Enhance local development mode for WaterUI
- enhance Dockerfile and documentation for improved build and configuration
- *(cli)* enhance Android backend integration with improved logging and automation

### Fixed

- update documentation links in README.md
- remove outdated contribution guidelines from README
- update README and Cargo.toml files to specify README.md for all components
- add placeholder crate to work around missing workspace member in old commit; update waterui version in README
- update documentation to reflect Android View terminology for consistency
- correct spelling errors and improve comments across the codebase

### Other

- Prepare dev for release automation
- Mark GTK pkg-config build dependency
- Fix GTK WebKit link probing
- Fix CI coverage and preview dylib targets
- Fix GTK resolved native views and Windows dylib linking
- Fix CI fontconfig deps and README doctests
- Clean dev CI and example builds
- Clean reactive view composition anti-patterns
- snapshot in-progress canonical changes
- Implement testing roadmap coverage and CI reporting
- Refactor split navigation around stable selection ids
- Checkpoint current WaterUI changes
- Reduce clippy noise across support crates
- Implement review fixes and native multi-date picker
- Refactor asset and icon build helpers
- Fix local state rebuild semantics
- Fix async filter setup handling
- Add semantic chart interaction testing
- Improve GTK backend completeness and filter support
- bridge view dimensions across backends
- checkpoint all in-progress workspace changes
- checkpoint all in-progress workspace changes
- commit pending workspace changes on dev
- *(raw-view)* migrate resolved color to native backends
- *(gtk)* align gpu surface redraw contract with request_redraw
- enforce fast-fail rules and remove legacy fallback paths
- commit full review and inspector/runtime updates
- Auto-install Meson on macOS when cargo build fails
- Remove on_demand and add needs_redraw
- Fix GTK main thread executor and video renderer
- Enhance video, chart, and locale platform support
- Remove hot reload functionality in favor of preview system
- Move static assets to R2 CDN and add PID-based window capture
- Refactor code for improved readability and consistency
- Linux-only GPU surface + WaterUI layout
- anyviews watch FFI + preview/gtk/window improvements
- Update android backend
- fix axis overlap and enable AA on HDR
- Add preview system and refactor core APIs
- Update backend submodules
- Fix the import of waterui-graphics
- Refactor color module to use waterui_graphics
- Remove Hydrolysis backend components and related infrastructure
- update licenses to include Apache 2.0 and MIT, and update README badge
- Release 0.2.0
- Bump waterui version to 0.2 in documentation across multiple components
- Add waterui-color, waterui-str, and waterui-url crates with comprehensive documentation
- Update README and FFI components for consistency and clarity
- Remove AGENT.md and enhance FFI bindings for events and gestures
- Remove terminal backend mention from README.md
- Update documentation and add CMake checks for Apple builds
- Make Suspense/hot reload use thread-safe executor
- update FFI header regeneration instructions and enhance README content
