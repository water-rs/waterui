# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/water-rs/waterui/releases/tag/waterui-mermaid-v0.1.0) - 2026-09-11

### Added

- *(mermaid)* claim mermaid fences in markdown
- *(mermaid)* draw Mermaid diagrams through Scene2D
- *(cli)* launcher icons for self-drawn desktop targets
- [**breaking**] close the issue #134-#148 audit sweep
- derive identifiable from id fields
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

- *(mermaid)* keep room around a flowchart's own geometry
- *(mermaid)* paint every label at the size it was measured at
- *(mermaid)* measure diagrams with the shared font collection
- *(mermaid)* draw the stadium as one closed outline
- fix repository rule violations and refresh documentation
- update documentation links in README.md
- remove outdated contribution guidelines from README
- update README and Cargo.toml files to specify README.md for all components
- add placeholder crate to work around missing workspace member in old commit; update waterui version in README
- update documentation to reflect Android View terminology for consistency
- correct spelling errors and improve comments across the codebase

### Other

- Merge pull request #442 from water-rs/agent/mermaid-canvas-padding-418
- *(mermaid)* take CodeConfig from waterui-text, not the root crate
- *(mermaid)* resolve the intra-doc link to ViewConfiguration::render
- Update README.md
- rewrite the README around the official logo
- prefer arrays for fixed collections
- rewrite root README
- Make reactivity precise across renderers
- fix a typo (so you now know i'm a human)
- Add AI policy and autonomous-agent guidance
- Move roadmap and changelog into docs
- Fix cargo doc warnings
- Prepare dev for release automation
- Fix CI fontconfig deps and README doctests
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
