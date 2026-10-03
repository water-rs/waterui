# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0](https://github.com/water-rs/waterui/compare/waterui-map-v0.1.2...waterui-map-v0.5.0) - 2026-09-20

### Added

- *(testing)* [**breaking**] split the test harness into a semantic pipeline and a styled rendered pipeline

### Fixed

- *(deps)* keep dev-deps on unreleased satellites out of published manifests ([#1124](https://github.com/water-rs/waterui/pull/1124))
- *(map)* install the GPU map realization in the semantics e2e

### Other

- drop dev-dependencies orphaned by the test moves; restore a trailing newline
- Revert "fix(deps): keep dev-deps on unreleased satellites out of published manifests ([#1124](https://github.com/water-rs/waterui/pull/1124))" ([#1129](https://github.com/water-rs/waterui/pull/1129))
- move the water CLI to water-rs/cli
- reorder suiteki imports ahead of waterui_*
- [**breaking**] replace waterui-str with the extracted suiteki crate

## [0.1.2](https://github.com/water-rs/waterui/compare/waterui-map-v0.1.1...waterui-map-v0.1.2) - 2026-09-11

### Other

- update Cargo.toml dependencies

## [0.1.1](https://github.com/water-rs/waterui/compare/waterui-map-v0.1.0...waterui-map-v0.1.1) - 2026-09-11

### Other

- updated the following local packages: waterui-core
