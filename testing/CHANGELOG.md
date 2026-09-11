# Changelog

All notable changes to `waterui-testing` are documented in this file.

## [Unreleased]

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
