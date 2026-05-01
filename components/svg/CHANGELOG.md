# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/water-rs/waterui/releases/tag/svg-v0.1.0) - 2026-05-01

### Added

- *(svg)* switch Svg to SceneView and share scene data parser
- continue deep review fixes and ffi fast-fail cleanup
- integrate assets, media, and runtime updates
- *(graphics)* Improve GPU surface and rendering pipeline
- Migrate embedded shaders to separate WGSL files
- Async GpuRenderer setup for flicker-free window display
- Introduce `waterui-canvas`, `waterui-shape`, and `waterui-svg` crates, refactor graphics components, and integrate Vello for advanced rendering.

### Fixed

- complete gpu surface fast-fail redraw and abi sync

### Other

- Prepare dev for release automation
- Add release crate READMEs
- Fix Android tooling and strict linting
- Clean dev CI and example builds
- Expand hydrolysis UI testing coverage
- Use imports for type paths
- Add view builder macros and migrate branchy views
- Extract image crate and tighten native rendering
- Reduce clippy noise across support crates
- bridge view dimensions across backends
- checkpoint all in-progress workspace changes
- enforce GpuView SubView impls and explicit GpuContext lifetimes
- add reactive redraw pipeline and unified resource drawing
- Implement Animatable pipeline and native filtered blur hooks
- add reactive redraw pipeline and unified resource drawing
- commit full review and inspector/runtime updates
- harden fast-fail paths and runtime compatibility
- Remove on_demand and add needs_redraw
- Refactor code for improved readability and consistency
