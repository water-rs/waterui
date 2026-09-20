# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0](https://github.com/water-rs/waterui/compare/layout-v0.3.2...layout-v0.5.0) - 2026-09-20

### Added

- *(layout)* [**breaking**] negotiate placement against the resolved bounds on both axes
- *(layout)* [**breaking**] propose the resolved cross extent at placement and freeze the contract
- *(layout)* alignment tokens and per-container alignment methods
- *(layout)* [**breaking**] retain negotiated proposals through placement

### Fixed

- *(layout)* report the child's floor on a min-size query in a max-only frame
- *(layout)* draw Divider as a hairline in the Border token colour
- negotiate constrained frame regions and finite stack offers
- *(layout)* preserve cross-axis responses and document conformance
- declare stretch_axis on composite views before body resolution ([#952](https://github.com/water-rs/waterui/pull/952))
- *(layout)* declare the View bound on every view-taking generic

### Other

- Merge pull request #1093 from water-rs/fix/remove-spacer-layout
- Merge pull request #1088 from water-rs/fix/spacer-min-length
- *(layout)* name the min-size query test without the typo
- Merge remote-tracking branch 'origin/dev' into feat/alignment-tokens
- *(layout)* inline private stack distribution buffers
- isolate layout unit tests from rendering backends ([#966](https://github.com/water-rs/waterui/pull/966))
- *(layout)* [**breaking**] drop the SafeAreaInsets contract

## [0.3.2](https://github.com/water-rs/waterui/compare/layout-v0.3.1...layout-v0.3.2) - 2026-09-11

### Other

- *(backends)* consume hydrolysis-m3 0.2.0 from crates.io and drop the in-tree copy ([#550](https://github.com/water-rs/waterui/pull/550))

## [0.3.1](https://github.com/water-rs/waterui/compare/layout-v0.3.0...layout-v0.3.1) - 2026-09-11

### Fixed

- *(layout)* measure a frame's child at the proposal, not the ideal
- *(layout)* propose the frame's resolved bounds to its child

### Other

- *(layout)* spell the child in the rigid-child test name

## [0.3.0](https://github.com/water-rs/waterui/compare/layout-v0.2.2...layout-v0.3.0) - 2026-08-25

### Changed

- Made stack compression and growth explicit through layout priority while preserving stretch intent through type erasure.

## [0.2.2](https://github.com/water-rs/waterui/compare/layout-v0.2.1...layout-v0.2.2) - 2025-12-14

### Fixed

- update README and Cargo.toml files to specify README.md for all components

## [0.2.1](https://github.com/water-rs/waterui/compare/layout-v0.2.0...layout-v0.2.1) - 2025-12-14

### Fixed

- add placeholder crate to work around missing workspace member in old commit; update waterui version in README

### Other

- update licenses to include Apache 2.0 and MIT, and update README badge
- Release 0.2.0
