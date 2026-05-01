# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/water-rs/waterui/releases/tag/waterui-particle-v0.1.0) - 2026-05-01

### Added

- *(particle)* add gpu neighbor grid interactions
- *(particle)* support multi-obstacle gpu collisions
- *(particle)* add gpu collision physics
- continue deep review fixes and ffi fast-fail cleanup
- *(particle)* Improve GPU particle system with encase
- introduce GPU particle system and refactor filtrate-core with new filter implementations and shader architecture

### Fixed

- *(particle)* preserve blending on hdr surfaces
- *(particle)* stabilize gpu offscreen rendering
- *(particle)* correct particle rendering basics
- complete gpu surface fast-fail redraw and abi sync

### Other

- Prepare dev for release automation
- Fix CI coverage and preview dylib targets
- Fix CI formatting and dependency checks
- Fix Android tooling and strict linting
- Clean dev CI and example builds
- Expand hydrolysis UI testing coverage
- *(particle)* move frame timing into renderer
- Bump dependency versions across workspace
- checkpoint all in-progress workspace changes
- enforce GpuView SubView impls and explicit GpuContext lifetimes
- commit pending workspace changes on dev
- commit full review and inspector/runtime updates
- Remove on_demand and add needs_redraw
- Move static assets to R2 CDN and add PID-based window capture
- Refactor code for improved readability and consistency
- Refactor handler API and modernize bindings
