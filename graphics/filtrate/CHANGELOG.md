# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **Breaking:** stages are functions for the shared composer,
  `cherenkov-shader`, instead of WGSL entry points. A colour stage is
  `fn apply(color, params…) -> color`; a spatial stage samples `input`
  through `input_point_sampler` or `input_sampler` at a normalized `uv`.
  Every built-in stage was converted, and filters that share a body now share
  one stage specialized with constants (box and gaussian blur axes, Sobel,
  Prewitt and edge work, perspective transform and correction, bloom and
  gloom).
- **Breaking:** filter kinds are types. `Filter::COLOR_ONLY` is replaced by
  `Filter::Kind` and the `ColorFilter` (`LINEAR`) and `SpatialFilter`
  (`footprint`) traits; `Chain<A, B>` is a colour filter exactly when both
  halves are.
- **Breaking:** `StageCollector` receives `ColorStage` and `SpatialStage`
  declarations, whose `ParamSource`s bind snippet parameters to filter
  parameters or constants, instead of source strings and parameter counts.
  `Blur`, `GaussianBlur`, `UnsharpMask`, `Bloom` and `Gloom` no longer repeat
  a parameter per pass.
- **Breaking:** luma comes from the working-space constants (linear Display
  P3) instead of hard-coded Rec. 709 coefficients, and colour stages operate
  on premultiplied colour.
- **Breaking:** `FilterAdapter` is replaced by `Executor`, a reference
  executor that composes a chain and runs one fragment pass per piece with
  `Rgba16Float` intermediates. `HdrPolicy`, `SpatialExecution`, the compute
  path, `EffectContext::shader_cache`, `Filter::output_size` and
  `Effect::output_size` are removed; input and output must have the same
  size.
- **Breaking:** the multi-input operations are ordinary filters in
  `filtrate::filters` (`BlendWithImage`, `MaskedBlur`, `ToneCurve`, …) whose
  images bind to their stage's auxiliary inputs; `MultiInputFilter`,
  `MultiInputOperation`, their aliases and constructor functions are removed,
  and `FilterImage` and `LutImage` move to the crate root.
- **Breaking:** `Vignette` is a spatial filter with a zero footprint, since
  it depends on the pixel's position.
- `EffectSetupError::InputNotFilterable` says the effect samples its input
  with a filtering sampler instead of naming the first stage, since a
  `ShaderEffect` raises it too.
- **Breaking:** `#[derive(Filter)]` takes `color` with `linear = <bool>` and
  an optional `cpu = <path>`, or `spatial` with `footprint = <f32>` or
  `footprint_fn = <path>` and an optional `shape`; both accept `space` and
  `constants`.

### Added

- Operating spaces: a stage declares `OperatingSpace::Srgb` and the executor
  converts around it.
- Shape input: a spatial stage can read the clip shape's signed distance
  field or mask, supplied through `EffectInput::shape`.
- SIMD CPU kernels (`CpuKernel`) for `Brightness`, `Saturation`,
  `Grayscale` and `ColorMatrix`, cross-checked against their shaders.
- `Executor::footprint` bounds a spatial chain's footprint over its running
  animations, through `AnimationTrack::magnitude_bound` and
  `Interpolator::bounds`.
- `ShaderEffect` runs an application-supplied WGSL fragment shader over an
  effect's input (a terminal's custom shader, a scanline or CRT pass). Its
  prelude declares `input_texture`, `input_sampler`, `uniforms` (output and
  input resolution, time, time delta, frame, parameter count, parameters) and
  `effect_param(i)`. `ShaderEffect::new` parses and validates the module at
  construction and returns a `ShaderEffectError` (`Parse`, `Validation`,
  `MissingEntryPoint`) with diagnostics in the application's own line
  numbers. Parameters are constant (`param`) or reactive (`watch_param`,
  which returns the subscription's `WatchGuard` to the caller), up to
  `SHADER_EFFECT_MAX_PARAMS`; `animated()` keeps the host drawing. The effect
  is `Send` and records into the host's shared encoder.

## [0.2.1](https://github.com/water-rs/filtrate/compare/v0.2.0...v0.2.1) - 2026-09-13

### Added

- add wasm-only WebGL2 backend via fragment spatial passes

### Fixed

- *(release)* trigger release on push to main
- normalize CRLF in spatial bodies before fragment translation

### Other

- *(release)* disable release-plz semver check for filtrate
- run semver preflight on default features only
- Merge pull request #6 from water-rs/feat/webgl-backend
