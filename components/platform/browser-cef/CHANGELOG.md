# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0](https://github.com/water-rs/waterui/compare/waterui-browser-cef-v0.2.1...waterui-browser-cef-v0.5.0) - 2026-09-20

### Added

- [**breaking**] #[state] marks owned types as extractors over .state()
- *(webview)* include_web! mounts a web frontend through the asset origin
- *(webview)* an interceptable local asset origin for bundled content

### Fixed

- clear the wasm32 and Windows nightly lint failures
- *(browser-cef)* derive the page request context from the browser host ([#1005](https://github.com/water-rs/waterui/pull/1005))
- *(cef)* wait for request-context initialization before creating a browser ([#949](https://github.com/water-rs/waterui/pull/949))
- apply rustfmt to the wave-2 engine sources

### Other

- Merge pull request #900 from water-rs/fix/cef-windows-bootstrap
- move the water CLI to water-rs/cli
- reorder suiteki imports ahead of waterui_*
- [**breaking**] replace waterui-str with the extracted suiteki crate

## [0.2.1](https://github.com/water-rs/waterui/compare/waterui-browser-cef-v0.2.0...waterui-browser-cef-v0.2.1) - 2026-09-11

### Other

- updated the following local packages: waterui-core, waterui-graphics, waterui-url, waterui-webview, waterui-chromium

## [0.2.0](https://github.com/water-rs/waterui/compare/waterui-browser-cef-v0.1.0...waterui-browser-cef-v0.2.0) - 2026-09-11

### Fixed

- *(hydrolysis)* make direct_to_target work on HiDPI, one format, opaque only

### Other

- *(deps)* vello_cpu 0.2 and the remaining tooling bumps
- *(cef)* take Chromium's message loop out of the render callback
- *(core)* give the default-accessibility-role rule one home
- *(webview)* decouple the Chromium CDP session from waterui-webview
