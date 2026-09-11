# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/water-rs/waterui/compare/waterui-dew-v0.2.0...waterui-dew-v0.2.1) - 2026-09-11

### Fixed

- *(dew)* place chrome-carved children at f64, not through an f32 frame ([#552](https://github.com/water-rs/waterui/pull/552))

## [0.2.0](https://github.com/water-rs/waterui/compare/waterui-dew-v0.1.0...waterui-dew-v0.2.0) - 2026-09-11

### Added

- *(text)* define the default type scale once
- *(text)* share one font collection through the environment
- *(graphics)* rasterise static scenes into a native Picture view
- *(graphics)* [**breaking**] give SceneContent an intrinsic size
- *(dew)* route interaction metadata to gesture and hover handlers
- render Scene2D content on dew's CPU rasterizer

### Fixed

- *(dew)* decide a navigation seam once, on layout's grid
- *(dew)* install dew's type scale for unset font slots
- *(release)* close package rehearsal gaps
- *(release)* verify registry-only package graph

### Other

- *(deps)* vello_cpu 0.2 and the remaining tooling bumps
- Merge pull request #445 from water-rs/agent/dependency-bumps-405
- Merge pull request #443 from water-rs/agent/dew-tab-bar-seam-441
- Merge pull request #437 from water-rs/agent/hydrolysis-script-fallback-426
- Merge pull request #398 from water-rs/agent/dew-a11y-labels-397
- Merge pull request #390 from water-rs/agent/dew-body-font-382
- Merge pull request #381 from water-rs/agent/dew-watch-sim-font-377
- Merge branch 'dev' into agent/scene-invalidator-watch-274/20260906-024222
- put visual-review exports in the canonical artifact layout
- export visual-review images under the shared artifact root
- *(chart)* consume waterui-chart 0.1.0 from crates.io and drop the in-tree copy
- *(canvas)* consume waterui-canvas 0.1.0 from crates.io and drop the in-tree copy
- *(release)* make workspace-internal dev-dependencies path-only
- Merge pull request #260 from water-rs/agent/dew-test-gating/20260902-102719
- ship the licence texts in every published crate
- prepare WaterUI 0.3 release versions
