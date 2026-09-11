# Changelog

All notable changes to `waterui-internal` are documented in this file.

## [Unreleased]

## [0.4.1](https://github.com/water-rs/waterui/compare/waterui-internal-v0.4.0...waterui-internal-v0.4.1) - 2026-09-11

### Added

- *(media)* let a photo preserve its aspect ratio inside the frame it is given ([#542](https://github.com/water-rs/waterui/pull/542))

## [0.4.0](https://github.com/water-rs/waterui/compare/waterui-internal-v0.3.0...waterui-internal-v0.4.0) - 2026-09-11

### Added

- *(text)* define the default type scale once
- *(waterui)* expose standalone components through the facade
- *(math)* speak the formula instead of announcing its MathML
- *(markdown)* make fenced code claimable and route FlowMarkdown through Code
- *(rich-text)* typeset Markdown math instead of dropping it

### Fixed

- *(web)* restore the wasm32 build of the hydrolysis web runner and check it in CI
- *(waterui-internal)* hoist a test helper above the statements it follows
- *(markdown)* make FlowMarkdownConfig a constant signal
- *(animation)* [**breaking**] merge the duplicate AnimationExt into one system-default trait
- *(release)* close package rehearsal gaps
- *(release)* verify registry-only package graph

### Other

- *(deps)* take the outstanding independent dependency releases
- Merge pull request #428 from water-rs/agent/kit-dialog-wasm-clippy-413
- *(wasm)* lint the wasm32 lane instead of only compiling it
- Merge pull request #275 from water-rs/agent/clippy-all-features-scope/20260902-131826
- *(text)* move Code into waterui-text so components can claim fences without depending on the root crate
- [**breaking**] depend on the self-drawn component crates directly
- [**breaking**] put syntect, pulldown-cmark, ICU formatters, bcrypt and regex behind features
- *(deps)* delete dependencies nothing imports
- [**breaking**] let the composition root install self-drawn realizations
- ship the licence texts in every published crate
- prepare WaterUI 0.3 release versions

## [0.3.0] - 2026-08-25

- Aligned the facade implementation and feature graph with the WaterUI 0.3 public release cohort.
