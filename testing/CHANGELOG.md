# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/water-rs/waterui/releases/tag/waterui-testing-v0.2.1) - 2026-05-01

### Added

- align focus state across backends
- *(runtime)* unify gesture semantics and picker backends
- *(graphics)* finalize advanced filter pipeline and core integration
- *(testing)* add waterui::test macro and a11y-first ui test runtime
- *(testing)* add headless waterui-testing host and snapshot smoke test
- integrate assets, media, and runtime updates
- introduce GPU texture filtering utility and image example, refactor media components
- *(README)* update header layout and add badges for improved visibility and branding
- *(README)* add WaterUI logo and update header for improved branding
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

- Fix Hydrolysis and testing strict lints
- Prepare dev for release automation
- Allow Hydrolysis testing adapters in test host
- Fix CI fontconfig deps and README doctests
- Fix Android tooling and strict linting
- Clean dev CI and example builds
- Clean reactive view composition anti-patterns
- Expand hydrolysis UI testing coverage
- Add view builder macros and migrate branchy views
- Restore extractor-based testing actions
- Stabilize waterui-testing gesture coverage
- Refine semantic handles in waterui-testing
- refactor handler state extraction
- Use hydrolysis headless runtime in testing
- Checkpoint current WaterUI changes
- Fix local state rebuild semantics
- Add cartesian chart selection and scrolling APIs
- Fix chart readout snapshots in offscreen testing
- Add semantic chart interaction testing
- Refine chart E2E testing support
- *(testing)* extract query builder and widen chart coverage
- checkpoint local workspace changes
- checkpoint all in-progress workspace changes
- add reactive redraw pipeline and unified resource drawing
- Implement Animatable pipeline and native filtered blur hooks
- add reactive redraw pipeline and unified resource drawing
- commit pending workspace changes on dev
- Remove on_demand and add needs_redraw
- Enhance video, chart, and locale platform support
- Remove hot reload functionality in favor of preview system
- Move static assets to R2 CDN and add PID-based window capture
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
