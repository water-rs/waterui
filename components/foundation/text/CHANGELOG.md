# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.1](https://github.com/water-rs/waterui/compare/text-v0.4.0...text-v0.4.1) - 2026-09-11

### Other

- *(backends)* consume hydrolysis-m3 0.2.0 from crates.io and drop the in-tree copy ([#550](https://github.com/water-rs/waterui/pull/550))

## [0.4.0](https://github.com/water-rs/waterui/compare/text-v0.3.0...text-v0.4.0) - 2026-09-11

### Added

- *(text)* define the default type scale once
- *(text)* share one font collection through the environment

### Fixed

- *(text)* draw Code from theme tokens and follow the colour scheme

### Other

- *(release)* make workspace-internal dev-dependencies path-only
- *(text)* move Code into waterui-text so components can claim fences without depending on the root crate

## [0.3.0](https://github.com/water-rs/waterui/compare/text-v0.2.2...text-v0.3.0) - 2026-08-25

### Added

- Added locale-aware CJK and RTL behavior, precise reactive text formatting, and reusable shaping caches.

## [0.2.2](https://github.com/water-rs/waterui/compare/text-v0.2.1...text-v0.2.2) - 2025-12-14

### Fixed

- update README and Cargo.toml files to specify README.md for all components

## [0.2.1](https://github.com/water-rs/waterui/compare/text-v0.2.0...text-v0.2.1) - 2025-12-14

### Fixed

- add placeholder crate to work around missing workspace member in old commit; update waterui version in README

### Other

- update licenses to include Apache 2.0 and MIT, and update README badge
- Release 0.2.0
