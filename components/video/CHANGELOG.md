# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/water-rs/waterui/releases/tag/video-v0.2.1) - 2026-05-01

### Added

- unify label semantics and menu runtimes
- unify label semantics and menu surfaces
- *(form)* add picker parity across native backends
- *(graphics)* finalize GPU-first filters and generators
- *(video)* expand fallback player policy and diagnostics
- continue deep review fixes and ffi fast-fail cleanup
- integrate assets, media, and runtime updates

### Fixed

- complete gpu surface fast-fail redraw and abi sync

### Other

- Fix PiP button lint on unsupported platforms
- Fix Hydrolysis and testing strict lints
- Prepare dev for release automation
- Fix video clippy on Linux
- Fix Android tooling and strict linting
- Clean dev CI and example builds
- Fix Android packaging toolchain resolution
- Fix reactive constant API inputs
- Simplify runtime player reactive composition
- Clean reactive view composition anti-patterns
- Expand hydrolysis UI testing coverage
- Use imports for type paths
- Add view builder macros and migrate branchy views
- Tighten reactive text updates and remove redundant AnyView
- refactor handler state extraction
- Fix reactive text and label misuse
- Reduce clippy noise across support crates
- bridge view dimensions across backends
- checkpoint all in-progress workspace changes
- checkpoint all in-progress workspace changes
- enforce GpuView SubView impls and explicit GpuContext lifetimes
- commit pending workspace changes on dev
- enforce fast-fail rules and remove legacy fallback paths
- commit full review and inspector/runtime updates
- ship rust fallback player updates and HDR test example
- Auto-install Meson on macOS when cargo build fails
- Remove on_demand and add needs_redraw
- Fix GTK main thread executor and video renderer
- Enhance video, chart, and locale platform support
