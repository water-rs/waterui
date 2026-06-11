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
to the panel. On a SPI/QSPI-bound display this is the difference between
26 fps full-frame refreshes and 60 fps local updates.

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
                       · RGB565 panel stream (embedded)
```

The interaction runtime (gestures, scrolling, frame economy) is shared
with other self-drawn backends through `waterui-backend-core`.

## Desktop panel simulator

The complete embedded rendering flow runs natively in a window — no
cross-compilation, the LVGL-SDL / Slint-preview pattern:

```bash
cargo run -p waterui-dew --example watch_sim --features simulator
```

`simulator::run(width, height, title, env, build_root, on_tick)` opens a
virtual panel of any size; `render_view_png` renders one frame headlessly
for snapshot tests.

## Status

Supported today: layout containers (`vstack`/`hstack`/`zstack`/padding),
colors, spacers, text (parley shaping; per-span styles pending), reactive
updates via `Binding`/`Computed` with dirty-region flushes. Unsupported
views fail fast with a clear panic rather than rendering incorrectly.

Embedded target status and the current Xtensa toolchain miscompilation
blocker are documented in `examples/embedded/dew-esp32s3/`.
