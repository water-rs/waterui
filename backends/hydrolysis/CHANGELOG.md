# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/water-rs/waterui/compare/hydrolysis-v0.1.0...hydrolysis-v0.2.0) - 2026-09-11

### Added

- *(text)* define the default type scale once
- *(text)* share one font collection through the environment
- give GpuView a backend-neutral input event vocabulary
- *(gpu-surface)* [**breaking**] give GpuFrame a first-class device scale

### Fixed

- *(web)* restore the wasm32 build of the hydrolysis web runner and check it in CI
- *(math)* publish the formula's MathML on its accessibility node
- *(graphics)* poll the device after every presented frame
- *(hydrolysis)* derive the output encoding from the target's format
- *(hydrolysis)* make direct_to_target work on HiDPI, one format, opaque only
- *(hydrolysis)* keep materialized views out of the address-keyed measure cache
- *(hydrolysis)* choose the scene engine from the adapter
- *(hydrolysis)* make the no-accessibility naming-scope shim const
- *(hydrolysis)* declare the test harness's native map bridge
- *(hydrolysis)* carry the a11y naming scope across an env snapshot
- *(animation)* [**breaking**] merge the duplicate AnimationExt into one system-default trait
- *(release)* close package rehearsal gaps
- *(release)* verify registry-only package graph

### Other

- Merge pull request #445 from water-rs/agent/dependency-bumps-405
- *(deps)* take the outstanding independent dependency releases
- *(wasm)* lint the wasm32 lane instead of only compiling it
- Merge pull request #371 from water-rs/agent/gpu-surface-reclaim-device-370
- *(hydrolysis)* capture filtered subtrees through a shared atlas
- *(canvas)* consume waterui-canvas 0.1.0 from crates.io and drop the in-tree copy
- *(release)* make workspace-internal dev-dependencies path-only
- Merge pull request #278 from water-rs/agent/direct-to-target-hidpi/20260902-140021
- [**breaking**] depend on the self-drawn component crates directly
- [**breaking**] select the WebView engine in the application, not the backend
- [**breaking**] own WPE input adaptation in waterui-browser-wpe
- [**breaking**] own CEF input adaptation in waterui-browser-cef
- [**breaking**] let the composition root install self-drawn realizations
- render embedded GPU surfaces on demand
- consolidate GPU glue into waterui-graphics helpers
- ship the licence texts in every published crate
- depend on shaderloom directly, and give the icon codegen its own name
- prepare WaterUI 0.3 release versions
