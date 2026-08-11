# waterui-dew

Dew is WaterUI's embedded-first CPU rendering backend: anti-aliased vector
UI on microcontrollers, with no GPU and no full-resolution framebuffer
required.

It pairs [`vello_cpu`](https://crates.io/crates/vello_cpu) sparse-strip
rasterization (the CPU sibling of the GPU renderer used by the hydrolysis
backend — same `kurbo` geometry, same `peniko` brushes) with a Slint-style
memory discipline: WaterUI's fine-grained reactivity identifies exactly
which view changed, the changed display-list commands identify the dirty
screen regions, and only those regions are re-rasterized — band by band,
through a scratch buffer no taller than `band_height` rows — then streamed
to the panel. On an SPI/QSPI-bound display, local updates avoid the bandwidth
ceiling imposed by full-frame transfers.

## Architecture

```
WaterUI view tree
   │  dispatch + waterui-layout measure/place
   ▼
DisplayList            retained draw commands (kurbo paths, peniko brushes)
   │  diff against previous frame → dirty rects
   ▼
BandScheduler          dirty rects → ≤ band_height row slices
   │
Painter (vello_cpu)    rasterize each band into a scratch pixmap
   │
DisplayFlush           the only platform-specific piece:
                       BufferDisplay (tests) · simulator window (desktop)
                       · Rgb565Display → simulated or physical band sink
```

Dew shares frame signals and input types with other self-drawn backends through
`waterui-backend-core`. `DewRuntime` routes retained pointer input to buttons,
toggles, sliders, and steppers without rebuilding their view bodies.

## Deliberately unsupported: the GPU stack

Dew's dependency graph is wgpu-free by design. The GPU-backed primitives —
`GpuSurface`, `ShaderSurface`, `ViewEffect`, and the `AppliedFilter` GPU
filters — are **explicitly unsupported** on Dew, not emulated: they require a
device the target class does not have, and a CPU re-implementation would be a
fallback pretending to be the real thing. A WaterUI app built for Dew must
not enable the `waterui/gpu` feature; a GPU view that reaches Dew's
dispatcher fails fast through `Native::body` rather than rendering a blank.

The same policy covers the styles Dew's widget handlers panic on
(`ButtonStyle`/`ToggleStyle`/`ProgressStyle` variants, non-vertical scroll
axes, indeterminate progress): each panic is an authored "not implemented
here" marker for a feature that needs a real Dew implementation, never a
silent degradation.

## Embedded-device simulator

The complete embedded rendering flow runs natively in a window — no
cross-compilation, the LVGL-SDL / Slint-preview pattern:

```bash
cargo run -p waterui-dew --example watch_sim --features embedded-simulator
```

`embedded_simulator::run(width, height, title, env, build_root, on_tick)` opens a
virtual panel of any size; `render_view_png` renders one frame headlessly
for snapshot tests.

## Performance gate

The ignored commercial-load test renders an interactive 480×320 vending-machine
screen with twelve product buttons, reactive order/payment text, and a
continuously updating progress bar. It constrains rasterization to the scalar
single-thread fallback, converts every rendered band through the same
`Rgb565Display` adapter used by ESP-IDF, accounts for transfers over a 40 MHz
SPI bus, and fails if any sampled frame exceeds the 60 Hz budget. Its simulated
panel retains no framebuffer; only one RGBA raster band and one RGB565 DMA band
exist at a time.

```bash
DEW_PERF_WARMUP=120 DEW_PERF_FRAMES=3600 cargo test -p waterui-dew \
  --test vending_performance vending_machine_holds_stable_sixty_fps \
  -- --ignored --exact
```

## Status

Supported today: layout containers (`vstack`/`hstack`/`zstack`/padding),
solid/gradient/image brushes, colors, spacers, styled text through parley,
reactive updates via `Binding`/`Computed`, retained pointer controls, and
dirty-region flushes. Unsupported views fail fast with a clear panic rather
than rendering incorrectly.

Without a selected physical board, the ESP-IDF entry is explicitly a headless
streaming simulation: it performs banded rasterization and RGB565 conversion
without allocating a framebuffer, then consumes each band. A physical board
implements `Rgb565Sink` to replace only that final sink; the Dew renderer,
memory discipline, and application remain unchanged.
