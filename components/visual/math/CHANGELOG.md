# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/water-rs/waterui/releases/tag/waterui-math-v0.1.0) - 2026-09-11

### Added

- *(math)* speak the formula instead of announcing its MathML
- *(text)* share one font collection through the environment
- *(graphics)* [**breaking**] give SceneContent an intrinsic size
- *(math)* add waterui-math, formula rendering on the OpenType MATH table

### Fixed

- *(math)* publish the formula's MathML on its accessibility node
- *(math)* redraw a bound formula when its source changes
- *(math)* satisfy the CI gates the local run did not reach

### Other

- Merge branch 'dev' into agent/scene-invalidator-watch-274/20260906-024222
- export visual-review images under the shared artifact root
- *(math)* add doctests for the public entry points
- *(math)* paint an opaque ground under the gallery renderings
- *(math)* render a formula gallery on both scene engines
