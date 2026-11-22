# waterui-conformance

Unified visual, perceptual, and interaction conformance suite for WaterUI.

## Capabilities

- **Cross-platform perceptual diff (NVIDIA FLIP)**: Pure-Rust LDR-FLIP over RGBA8 PNGs (`flip_mean`, `flip_p95`, `flip_max`, `max_block_flip_mean`, `max_local_error`), magma error heatmaps, and blank-frame detection with zero platform-specific image decoders.
- **Declarative showcase runner**: `RenderCase`, `ShowcaseSuite`, and `GalleryRunner` built on `waterui-testing` with deterministic viewports, scale factors, light/dark appearances, multi-stage interactions, semantic assertions, and canonical `<suite>/<case>/<stage>.png` artifacts under `WATERUI_TEST_ARTIFACTS_DIR`.
- **Static gallery artifacts**: Emits machine-readable `report.json` alongside a self-contained static `index.html` gallery for side-by-side, toggle, and heatmap inspection.
- **Standard conformance suites**: `waterui_standard_suite()` for core WaterUI components and `hydrolysis_style_suite<S: Style>()` for validating Hydrolysis style packages across light/dark appearances and interactive states.
- **Cross-backend `ViewRenderer` parity**: Per-component offscreen parity runner (`ParityRunner`) comparing any two `ViewRenderer` backends with FLIP metrics and magma heatmaps.

## Features

- `default = ["testing", "gpu"]`
- `capture`: Enables `ParityRunner` over `waterui-capture::ViewRenderer` without pulling in `waterui-testing`.
- `testing`: Enables `ShowcaseSuite`, `GalleryRunner`, `waterui_standard_suite()`, and `hydrolysis_style_suite()`.
- `gpu` / `cpu`: Selects the Hydrolysis offscreen painter(s) forwarded to `waterui-testing`.
- `--no-default-features`: Pure-Rust FLIP engine, blank-frame detector, magma heatmap encoder, and `report.json` / `index.html` gallery generator.
