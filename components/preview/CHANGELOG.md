# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/water-rs/waterui/releases/tag/waterui-preview-v0.1.0) - 2026-05-01

### Added

- ship gpu on-demand rendering, animation updates, and cli/runtime sync
- integrate assets, media, and runtime updates
- *(preview)* Add water preview command for view rendering
- introduce GPU texture filtering utility and image example, refactor media components
- *(README)* update header layout and add badges for improved visibility and branding
- *(README)* add WaterUI logo and update header for improved branding
- Enhance window management and hot reload functionality
- Revamp README and enhance Android hot reload functionality
- Enhance local development mode for WaterUI
- enhance Dockerfile and documentation for improved build and configuration
- *(cli)* enhance Android backend integration with improved logging and automation

### Fixed

- *(preview)* harden symbol resolution and render error handling
- update documentation links in README.md
- remove outdated contribution guidelines from README
- update README and Cargo.toml files to specify README.md for all components
- add placeholder crate to work around missing workspace member in old commit; update waterui version in README
- update documentation to reflect Android View terminology for consistency
- correct spelling errors and improve comments across the codebase

### Other

- Prepare dev for release automation
- Fix CI coverage and preview dylib targets
- Fix CI fontconfig deps and README doctests
- Clean dev CI and example builds
- split blocking queue and dlopen timing
- expose support app render timings
- trim crate-type override and persist support logs
- trim dylib debug info and init support tracing
- move rust stdlib rpath to build and drop runtime dylib patching
- use local-path render in CLI and unify HasDylib cache truth
- use local dylib path and extend support-app idle lifetime
- enforce dev dynamic-linking and split runtime wrapper crates
- Optimize preview support app launch and hot path
- Clean reactive view composition anti-patterns
- unify cache and support app helpers
- Reduce clippy noise across support crates
- Bump dependency versions across workspace
- enforce fast-fail rules and remove legacy fallback paths
- commit full review and inspector/runtime updates
- Remove on_demand and add needs_redraw
- Enhance video, chart, and locale platform support
- Remove hot reload functionality in favor of preview system
- Move static assets to R2 CDN and add PID-based window capture
- Refactor preview app communication and remove unresponsive app handling
- Enhance preview app communication and error handling
- Refactor code for improved readability and consistency
- anyviews watch FFI + preview/gtk/window improvements
- Integrate preview TCP server into view
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
