# Cherenkov

A GPU 2D rendering engine in Rust, built for modern hardware.

Cherenkov is being designed to render WaterUI's self-drawn backend, Hydrolysis, in place of Vello, and later to serve as the 2D engine of a web browser. It targets current GPUs only.

- **Hardware floor.** On-chip programmable blending must be available: raster order groups and imageblocks on Metal; rasterization-order attachment access or dynamic-rendering local read on Vulkan.
- **The raster architecture is chosen by measurement.** Where geometry is expanded (CPU, vertex, compute or mesh), who processes pixels (fragment, a tile interpreter or compute), and where blending dependencies resolve (fixed function, ordered on-chip, explicit epochs or exclusive ownership) are experimental axes. The cross-engine device farm (#3) decides between them.
- **Colour.** Rendering happens in linear, extended-range colour with a wide-gamut working space and native HDR output.
- **Layers.** The engine keeps a retained layer tree, and animates layer properties itself at present time.
- **Semantic primitives are first-class.** Rounded rectangles with continuous corners, shadows, glyph runs, images and gradients are drawn directly, not lowered to generic paths.

The CPU side is data-parallel and SIMD-vectorised, and covered by microbenchmarks.

## Status

The scene corpus (`scenes/corpus`), perf scenes (`scenes/perf`) and
fonts (`scenes/fonts`) are generated, not committed:
`python3 scenes/tools/generate.py` produces the whole tree
(`python3 -m pip install -r scenes/fonts/tools/requirements.txt`
first).

The project is in the design phase and there is nothing to use yet. The public API is designed first, followed by a cross-engine correctness and performance suite that measures Cherenkov against Skia and Vello.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
