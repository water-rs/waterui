# Changelog

All notable changes to `waterui-core` are documented in this file.

## [Unreleased]

## [0.5.0](https://github.com/water-rs/waterui/compare/core-v0.3.2...core-v0.5.0) - 2026-09-20

### Added

- [**breaking**] #[state] marks owned types as extractors over .state()
- *(layout)* [**breaking**] retain negotiated proposals through placement
- *(core)* add on_unimplemented diagnostics to first-contact traits
- *(a11y)* add the Dialog accessibility role ([#653](https://github.com/water-rs/waterui/pull/653))
- *(a11y)* [**breaking**] add a value channel to accessibility nodes

### Fixed

- declare stretch_axis on composite views before body resolution ([#952](https://github.com/water-rs/waterui/pull/952))
- *(cli)* install the app's translation catalog at every generated boundary

### Other

- *(layout)* inline scratch storage for small child sets
- reorder suiteki imports ahead of waterui_*
- [**breaking**] replace waterui-str with the extracted suiteki crate
- Merge pull request #635 from water-rs/feat/a11y-value-channel-457

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
