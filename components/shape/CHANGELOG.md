# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/water-rs/waterui/releases/tag/waterui-shape-v0.1.0) - 2026-05-01

### Added

- complete phase 0b and phase 1+2r foundation
- continue deep review fixes and ffi fast-fail cleanup
- ship gpu on-demand rendering, animation updates, and cli/runtime sync
- integrate assets, media, and runtime updates
- *(graphics)* Improve GPU surface and rendering pipeline
- introduce GPU particle system and refactor filtrate-core with new filter implementations and shader architecture
- Async GpuRenderer setup for flicker-free window display
- introduce GPU texture filtering utility and image example, refactor media components
- Introduce `waterui-canvas`, `waterui-shape`, and `waterui-svg` crates, refactor graphics components, and integrate Vello for advanced rendering.

### Fixed

- complete gpu surface fast-fail redraw and abi sync
- *(graphics)* robust pipeline cache retry logic for renderers

### Other

- Prepare dev for release automation
- Fix Android tooling and strict linting
- Clean dev CI and example builds
- Expand hydrolysis UI testing coverage
- Use imports for type paths
- Reduce clippy noise across support crates
- Bump dependency versions across workspace
- checkpoint all in-progress workspace changes
- enforce GpuView SubView impls and explicit GpuContext lifetimes
- commit pending workspace changes on dev
- commit full review and inspector/runtime updates
- Auto-install Meson on macOS when cargo build fails
- Remove on_demand and add needs_redraw
- Enhance video, chart, and locale platform support
- Refactor code for improved readability and consistency
