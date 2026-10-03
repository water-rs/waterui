# Changelog

All notable changes to `waterui-testing` are documented in this file.

## [Unreleased]

## [0.5.2](https://github.com/water-rs/waterui/compare/waterui-testing-v0.5.1...waterui-testing-v0.5.2) - 2026-09-22

### Other

- updated the following local packages: waterui

## [0.5.1](https://github.com/water-rs/waterui/compare/waterui-testing-v0.5.0...waterui-testing-v0.5.1) - 2026-09-22

### Other

- update Cargo.toml dependencies

## [0.5.0](https://github.com/water-rs/waterui/compare/waterui-testing-v0.4.1...waterui-testing-v0.5.0) - 2026-09-20

### Added

- *(testing)* styled builder mount_app honours viewport, flavor and scale factor
- *(testing)* a styled builder's semantic mount installs the style's tokens
- *(testing)* [**breaking**] split the test harness into a semantic pipeline and a styled rendered pipeline
- [**breaking**] #[state] marks owned types as extractors over .state()
- *(mcp)* close the testing-surface parity gaps
- *(a11y)* add the Dialog accessibility role ([#653](https://github.com/water-rs/waterui/pull/653))

### Fixed

- *(deps)* keep dev-deps on unreleased satellites out of published manifests ([#1124](https://github.com/water-rs/waterui/pull/1124))
- *(testing)* wait for in-flight work while settling; queue presses for transients
- *(testing)* stop settling on live producers and hold the clock while pacing
- *(preview)* wait for parked local work before a one-shot render
- *(testing)* settle waits on tasks parked on wall-clock I/O
- *(map)* install the GPU map realization in the semantics e2e
- *(testing)* install the self-drawn video realization on every host
- *(testing)* make Query must_use so a builder chain cannot be a no-op

### Other

- drop dev-dependencies orphaned by the test moves; restore a trailing newline
- *(testing)* one semantic mount path behind both builders
- Revert "fix(deps): keep dev-deps on unreleased satellites out of published manifests ([#1124](https://github.com/water-rs/waterui/pull/1124))" ([#1129](https://github.com/water-rs/waterui/pull/1129))
- *(testing)* pass the device-loss handle in the scene-view render test
- *(snapshot)* hand hydrolysis the offscreen surface's device-loss handle
- isolate layout unit tests from rendering backends ([#966](https://github.com/water-rs/waterui/pull/966))
- step through animations on the virtual frame clock
- Merge pull request #695 from water-rs/agent/focus-tests-687
- serve a mounted OffscreenApp to an agent over MCP ([#692](https://github.com/water-rs/waterui/pull/692))
- mount a whole App on the application runtime ([#683](https://github.com/water-rs/waterui/pull/683))

## [0.4.1](https://github.com/water-rs/waterui/compare/waterui-testing-v0.4.0...waterui-testing-v0.4.1) - 2026-09-11

### Other

- *(backends)* consume hydrolysis 0.2.0 from crates.io and drop the in-tree copy ([#553](https://github.com/water-rs/waterui/pull/553))

## [0.4.0](https://github.com/water-rs/waterui/compare/waterui-testing-v0.3.0...waterui-testing-v0.4.0) - 2026-09-11

### Added

- *(graphics)* [**breaking**] give SceneContent an intrinsic size
- give GpuView a backend-neutral input event vocabulary

### Fixed

- *(testing)* install a complete theme under ui() by default
- *(hydrolysis)* choose the scene engine from the adapter
- *(testing)* install self-drawn realizations in the test harness env
- *(release)* close package rehearsal gaps

### Other

- *(deps)* take the outstanding independent dependency releases
- *(canvas)* consume waterui-canvas 0.1.0 from crates.io and drop the in-tree copy
- *(release)* make workspace-internal dev-dependencies path-only
- [**breaking**] depend on the self-drawn component crates directly
- *(deps)* turn off default features nothing in the workspace uses
- [**breaking**] own WPE input adaptation in waterui-browser-wpe
- [**breaking**] ungate Scene2D from the GPU stack and drop its Vello escape hatches
- ship the licence texts in every published crate
- prepare WaterUI 0.3 release versions
- prepare WaterUI 0.3 release guidance
- prepare publishable dependency graph

## [0.3.0] - 2026-08-25

- Added Hydrolysis accessibility-tree queries, interactions, condition-based waits, and offscreen rendering coverage.
