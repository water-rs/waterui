# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/water-rs/waterui/releases/tag/waterui-chart-v0.1.0) - 2026-05-01

### Added

- *(chart)* add interactive canvas gestures
- *(chart)* migrate chart views to canvas scene pipeline
- continue deep review fixes and ffi fast-fail cleanup
- ship gpu on-demand rendering, animation updates, and cli/runtime sync
- integrate assets, media, and runtime updates
- *(chart)* Add chart component crate
- introduce GPU texture filtering utility and image example, refactor media components
- *(README)* update header layout and add badges for improved visibility and branding
- *(README)* add WaterUI logo and update header for improved branding
- Enhance window management and hot reload functionality
- Revamp README and enhance Android hot reload functionality
- Enhance local development mode for WaterUI
- enhance Dockerfile and documentation for improved build and configuration
- *(cli)* enhance Android backend integration with improved logging and automation

### Fixed

- complete gpu surface fast-fail redraw and abi sync
- update documentation links in README.md
- remove outdated contribution guidelines from README
- update README and Cargo.toml files to specify README.md for all components
- add placeholder crate to work around missing workspace member in old commit; update waterui version in README
- update documentation to reflect Android View terminology for consistency
- correct spelling errors and improve comments across the codebase

### Other

- Fix chart test lint issues
- Prepare dev for release automation
- Keep chart readout shell inside viewport
- Fix CI fontconfig deps and README doctests
- Fix CI formatting and dependency checks
- Fix Android tooling and strict linting
- Clean dev CI and example builds
- Fix Android packaging toolchain resolution
- Clean reactive view composition anti-patterns
- Expand hydrolysis UI testing coverage
- Add view builder macros and migrate branchy views
- Tighten reactive text updates and remove redundant AnyView
- refactor handler state extraction
- Unify semantic text and label inputs
- Checkpoint current WaterUI changes
- Reduce clippy noise across support crates
- Implement review fixes and native multi-date picker
- Add cartesian chart selection and scrolling APIs
- Add chart tooltip overlay orchestration
- Add scoped local state for chart composition overlays
- Fix chart readout snapshots in offscreen testing
- Remove legacy chart test support entry
- Add semantic chart interaction testing
- Refine chart E2E testing support
- *(testing)* extract query builder and widen chart coverage
- checkpoint all in-progress workspace changes
- Bump dependency versions across workspace
- checkpoint all in-progress workspace changes
- commit pending workspace changes on dev
- *(str)* migrate chart, drag-drop, and ime paths to Str
- *(chart)* remove legacy wgsl renderer stack
- commit full review and inspector/runtime updates
- Remove on_demand and add needs_redraw
- Enhance video, chart, and locale platform support
- Remove hot reload functionality in favor of preview system
- Move static assets to R2 CDN and add PID-based window capture
- Refactor code for improved readability and consistency
- fix axis overlap and enable AA on HDR
- Refactor handler API and modernize bindings
- Add preview system and refactor core APIs
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
