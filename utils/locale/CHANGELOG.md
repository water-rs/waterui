# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0](https://github.com/water-rs/waterui/compare/locale-v0.1.2...locale-v0.5.0) - 2026-09-20

### Fixed

- *(locale)* schedule regional refresh without a native thread on wasm ([#1111](https://github.com/water-rs/waterui/pull/1111))
- *(locale)* apply width, fill and Debug to localised text arguments

### Other

- reorder suiteki imports ahead of waterui_*
- [**breaking**] replace waterui-str with the extracted suiteki crate
- merge main (0.4.1 release commits) back into dev
- *(deps)* move the requirements the extraction left behind ([#557](https://github.com/water-rs/waterui/pull/557))

## [0.1.2](https://github.com/water-rs/waterui/compare/locale-v0.1.1...locale-v0.1.2) - 2026-09-11

### Other

- update Cargo.toml dependencies

## [0.1.1](https://github.com/water-rs/waterui/compare/locale-v0.1.0...locale-v0.1.1) - 2026-09-11

### Other

- updated the following local packages: waterui-core
