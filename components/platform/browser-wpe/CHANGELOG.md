# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0](https://github.com/water-rs/waterui/compare/waterui-browser-wpe-v0.1.2...waterui-browser-wpe-v0.5.0) - 2026-09-20

### Added

- *(webview)* an interceptable local asset origin for bundled content

### Fixed

- *(browser-wpe)* build the runtime without the DRM display
- *(browser-wpe)* disable JPEG XL in the runtime build (no libjxl on jammy)
- apply rustfmt to the wave-2 engine sources

### Other

- *(browser-wpe)* delete the dead WPE real-engine test target
- move the water CLI to water-rs/cli
- reorder suiteki imports ahead of waterui_*
- [**breaking**] replace waterui-str with the extracted suiteki crate

## [0.1.2](https://github.com/water-rs/waterui/compare/waterui-browser-wpe-v0.1.1...waterui-browser-wpe-v0.1.2) - 2026-09-11

### Other

- updated the following local packages: waterui-core, waterui-graphics, waterui-url, waterui-webview

## [0.1.1](https://github.com/water-rs/waterui/compare/waterui-browser-wpe-v0.1.0...waterui-browser-wpe-v0.1.1) - 2026-09-11

### Fixed

- *(hydrolysis)* make direct_to_target work on HiDPI, one format, opaque only

### Other

- *(core)* give the default-accessibility-role rule one home
- *(webview)* decouple the Chromium CDP session from waterui-webview
