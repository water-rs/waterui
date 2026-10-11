# Changelog

All notable changes to `waterui-webview` are documented in this file.

## [Unreleased]

## [0.5.0](https://github.com/water-rs/waterui/compare/waterui-webview-v0.4.1...waterui-webview-v0.5.0) - 2026-09-20

### Added

- *(testing)* [**breaking**] split the test harness into a semantic pipeline and a styled rendered pipeline
- [**breaking**] #[state] marks owned types as extractors over .state()
- *(webview)* include_web! mounts a web frontend through the asset origin
- *(webview)* an interceptable local asset origin for bundled content

### Fixed

- *(deps)* keep dev-deps on unreleased satellites out of published manifests ([#1124](https://github.com/water-rs/waterui/pull/1124))
- apply rustfmt to the wave-2 engine sources
- follow the suiteki Str migration and the no-engine controller removal

### Other

- Revert "fix(deps): keep dev-deps on unreleased satellites out of published manifests ([#1124](https://github.com/water-rs/waterui/pull/1124))" ([#1129](https://github.com/water-rs/waterui/pull/1129))
- move the water CLI to water-rs/cli
- *(webview)* [**breaking**] remove the no-engine WebViewController
- reorder suiteki imports ahead of waterui_*
- [**breaking**] replace waterui-str with the extracted suiteki crate

## [0.4.1](https://github.com/water-rs/waterui/compare/waterui-webview-v0.4.0...waterui-webview-v0.4.1) - 2026-09-11

### Other

- *(backends)* consume hydrolysis-m3 0.2.0 from crates.io and drop the in-tree copy ([#550](https://github.com/water-rs/waterui/pull/550))
- Merge remote-tracking branch 'gh/main' into agent/merge-release-0.4

## [0.4.0](https://github.com/water-rs/waterui/compare/waterui-webview-v0.3.0...waterui-webview-v0.4.0) - 2026-09-11

### Added

- *(webview)* [**breaking**] specify the raw run_javascript reply as the JSON encoding of the value

### Other

- *(core)* give the default-accessibility-role rule one home
- *(webview)* decouple the Chromium CDP session from waterui-webview

## [0.3.0](https://github.com/water-rs/waterui/compare/webview-v0.2.0...webview-v0.3.0) - 2026-08-25

- Made navigation asynchronous, required explicit URL parsing, and added origin-checked message handling.
