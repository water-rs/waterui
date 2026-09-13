# Changelog

All notable changes to `waterui-core` are documented in this file.

## [Unreleased]

## [0.3.2](https://github.com/water-rs/waterui/compare/core-v0.3.1...core-v0.3.2) - 2026-09-11

### Other

- update Cargo.toml dependencies

## [0.3.1](https://github.com/water-rs/waterui/compare/core-v0.3.0...core-v0.3.1) - 2026-09-11

### Other

- *(core)* give the default-accessibility-role rule one home

## [0.3.0](https://github.com/water-rs/waterui/compare/core-v0.2.0...core-v0.3.0) - 2026-08-25

### Added

- Added precise signal-driven view inputs, accessibility identifiers, theme tokens, safe-area metadata, and renderer inspection contracts.

### Changed

- Removed renderer-owned local state slots and made mutable UI state explicit through `Binding` and `Computed` values.
- Updated reactivity to Nami 0.11 and preserved layout stretch intent through erased views.
