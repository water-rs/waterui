# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/water-rs/waterui/releases/tag/waterui-internal-v0.2.1) - 2026-05-01

### Added

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

- Prepare dev for release automation
- Fix CI fontconfig deps and README doctests
- Clean dev CI and example builds
- enforce dev dynamic-linking and split runtime wrapper crates
- Clean reactive view composition anti-patterns
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
