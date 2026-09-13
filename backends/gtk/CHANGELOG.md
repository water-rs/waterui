# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.2](https://github.com/water-rs/waterui/compare/waterui-gtk-v0.1.1...waterui-gtk-v0.1.2) - 2026-09-11

### Other

- updated the following local packages: waterui-core, waterui-graphics, waterui-layout, waterui-locale, waterui-text, waterui-icon, waterui-controls, waterui-assets, waterui-shape, waterui-shape, waterui-form, waterui-navigation, waterui-webview, waterui, waterui-backend-core

## [0.1.1](https://github.com/water-rs/waterui/compare/waterui-gtk-v0.1.0...waterui-gtk-v0.1.1) - 2026-09-11

### Added

- *(graphics)* let a picture offer its own accessible name
- *(text)* share one font collection through the environment

### Fixed

- *(gtk)* clip shapes from their ShapeKind, not from path commands
- *(waterui-gtk)* make the WebKit-absent stubs diverge without dead bindings

### Other

- Merge pull request #388 from water-rs/agent/gtk-shape-kind-clip-157
