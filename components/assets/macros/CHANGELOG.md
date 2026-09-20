# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0](https://github.com/water-rs/waterui/compare/waterui-assets-macros-v0.1.2...waterui-assets-macros-v0.5.0) - 2026-09-20

### Added

- *(webview)* include_web! mounts a web frontend through the asset origin
- *(assets)* [**breaking**] include_bundle! generates its mount and reports it through artifact metadata

### Fixed

- *(assets-macros)* report io::ErrorKind instead of the OS error text

### Other

- stop the nightly matrix from repeating feature-independent work
- move the water CLI to water-rs/cli
- *(assets)* extract waterui-assets-core lightweight crate

## [0.1.2](https://github.com/water-rs/waterui/compare/waterui-assets-macros-v0.1.1...waterui-assets-macros-v0.1.2) - 2026-09-11

### Other

- updated the following local packages: waterui-assets, waterui-assets-planner

## [0.1.1](https://github.com/water-rs/waterui/compare/waterui-assets-macros-v0.1.0...waterui-assets-macros-v0.1.1) - 2026-09-11

### Other

- updated the following local packages: waterui-assets, waterui-assets-planner
