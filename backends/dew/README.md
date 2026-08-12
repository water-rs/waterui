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

## Performance gate: a work simulation, not a benchmark

The vending-machine test renders an interactive 480×320 screen with twelve
product buttons, reactive order/payment text, and a continuously updating
progress bar, single-threaded through the scalar fallback rasterizer and the
same `Rgb565Display` adapter ESP-IDF uses. Its simulated panel retains no
framebuffer: one RGBA raster band and one RGB565 DMA band exist at a time.

What it asserts on is the point. **Host wall-clock is recorded but is never a
pass criterion.** A development machine runs a different instruction set at
roughly twenty times the clock, and it executes `kurbo`'s `f64` geometry in
hardware where every ESP32-class FPU is single-precision and emulates it in
software — so the host is disproportionately fast at exactly the arithmetic Dew
does most. No constant converts one to the other.

What transfers exactly is the *amount of work*: how many text runs get shaped,
how many glyph outlines get read, how many measure calls layout makes, how many
commands the painter revisits per band, how many pixels are rasterized and
pushed down the bus. Those counts are identical on host and target because they
are properties of the algorithm rather than of the machine. `stats::FrameWork`
carries them, `DewRuntime::pump` returns them, and the test fails on:

- a per-frame **work budget** exceeded,
- **heap retained** across steady-state frames — the failure mode most likely
  to kill a real port, since a few hundred KiB of SRAM is unforgiving,
- more than **one band of pixels** handed to the board at once.

None of that depends on host speed, so the test is deterministic and runs in
CI rather than being `#[ignore]`d.

```bash
cargo nextest run -p waterui-dew -E 'test(vending_machine_holds_its_embedded_work_budget)'

# Longer soak, for heap-retention confidence.
DEW_PERF_WARMUP=120 DEW_PERF_FRAMES=3600 \
  cargo nextest run -p waterui-dew -E 'test(vending_machine_holds)'
```

Each run writes `/tmp/waterui_dew_vending_performance.toml` with the full work
vector, cache hit rates, memory figures, and panel-bus arithmetic.

### The projected on-chip time is a projection

The report also carries a projected per-frame time for a named chip, derived
from `ChipBudget` — a small table of per-operation cycle costs. Those costs are
`Provenance::Estimated`: inferred from clock rates and rough instruction counts,
good for ranking two designs and not for promising a frame rate. Every report
says so in its own text. Calibrating the table against on-device cycle counters
and flipping the provenance to `Measured` turns every existing simulation into a
real time estimate, with no other change anywhere.

### The bus is usually the ceiling

480×320 RGB565 over 40 MHz SPI is ~65 ms per full frame — about 15 FPS — no
matter how fast the chip is. Dew's dirty-region design exists to stay off that
ceiling, and `full_screen_repaint_is_bus_bound_below_thirty_fps` pins the
assumption so it cannot rot. Any full-screen change (a scroll, a page
transition) stays bus-bound on SPI; a parallel RGB/i80 or MIPI-DSI panel is what
removes the limit.

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
