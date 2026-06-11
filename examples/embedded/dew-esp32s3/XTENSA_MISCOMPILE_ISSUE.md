# Draft: esp-rs/rust issue — Xtensa backend miscompiles vello_cpu sparse-strip rasterization (any anti-aliased fill)

> Submit to: https://github.com/esp-rs/rust/issues
> Suggested title: `Miscompilation on xtensa-esp32s3-espidf: vello_cpu rect fill produces corrupted slice lengths / wild loads (works on wasm32 + all other targets)`

## Summary

Rendering a single rectangle with [`vello_cpu`](https://crates.io/crates/vello_cpu) (pure safe Rust, CPU rasterizer by Linebender) crashes on `xtensa-esp32s3-espidf`, on both real hardware (ESP32-S3, Waveshare Touch-AMOLED-2.06) and Espressif QEMU. The same code, same crate versions, same profile settings run correctly on `aarch64-apple-darwin` (including with the `fearless_simd` scalar `Fallback` level forced) and on `wasm32-wasip1` under wasmtime — which is also a 32-bit target taking the same scalar fallback code path. The only remaining variable is the Xtensa backend.

## Minimal reproduction

```rust
use kurbo::Shape;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let mut resources = vello_cpu::Resources::new();
    let mut pixmap = vello_cpu::Pixmap::new(96, 16);

    // Empty scene renders fine:
    let ctx = vello_cpu::RenderContext::new(96, 16);
    ctx.render_to_pixmap(&mut resources, &mut pixmap);
    log::info!("PROBE_EMPTY_OK"); // reached

    // One rect fill crashes:
    let mut ctx = vello_cpu::RenderContext::new(96, 16);
    ctx.set_paint(peniko::Color::from_rgb8(200, 30, 30));
    ctx.fill_path(&kurbo::Rect::new(0.0, 0.0, 96.0, 16.0).to_path(0.05));
    ctx.flush();
    ctx.render_to_pixmap(&mut resources, &mut pixmap); // crash
    log::info!("PROBE_ONE_FILL_OK"); // never reached
}
```

Dependencies: `vello_cpu = "0.0.9"`, `kurbo = "0.13"`, `peniko = "0.6"`, `esp-idf-svc = "0.52"` (ESP-IDF v5.3.3, std). Profile used:

```toml
[profile.dev]
opt-level = 2          # also reproduces at 1, "s", and with lto = "fat"
debug-assertions = false
overflow-checks = false
```

## Failure modes observed

Depending on toolchain/opt-level, the same logical failure surfaces as:

- `Guru Meditation Error: Core 0 panic'ed (LoadProhibited)` with `EXCVADDR: 0x00000008` (a dangling/null-page load) inside
  `vello_cpu::fine::lowp` blend iterators (`Zip<ChunksExactMut<u8>, Map<Copied<Iter<[u8; 8]>>, …>>`) — 1.95.0.0, opt 1/2;
- a *clean* Rust panic `index out of bounds: the len is 0 but the index is 0` at `vello_cpu-0.0.9/src/fine/mod.rs:567`
  (`attrs.fill[s.attrs_idx as usize]`) — 1.96.0.0, opt "s";
- a *clean* panic in `bytemuck::internal::cast_slice::<u8, [u8; 8]>` (slice length not divisible by 8) — 1.96.0.0 with `lto = "fat"`, and vello git main.

I.e. the coarse rasterization stage (`vello_common` strip generation) produces corrupted lengths/indices, and at higher opt levels even the resulting bounds checks misbehave (wild load instead of a panic). At `opt-level = 3` the *compiler itself* crashes with `rustc-LLVM ERROR: Cannot select: … i32 = Constant<-4096>` / SIGSEGV in `XtensaSizeReduce::ReduceMIE` while compiling a downstream crate, which may or may not be related.

## Evidence matrix

| Variable | Tried | Result |
| --- | --- | --- |
| Hardware vs emulator | ESP32-S3 board (USB-Serial-JTAG) and Espressif QEMU 9.2.2 | both crash identically |
| Toolchain | esp 1.95.0.0, esp 1.96.0.0 | both fail (failure shape differs) |
| Opt level | 1, 2, "s", fat LTO (0 fails to link: `l32r: literal target out of range`) | all fail |
| vello_cpu pipeline | u8 (`OptimizeSpeed`) and f32 (`OptimizeQuality`, `f32_pipeline` feature) | both fail |
| vello_cpu version | 0.0.9 and git main (`da699de`) | both fail |
| Same code on `wasm32-wasip1` (32-bit usize, scalar `Fallback` SIMD level, same profile) | wasmtime | **passes** |
| Same code on `aarch64-apple-darwin` with `Level::Fallback` forced (`fearless_simd` `force_support_fallback`) | both pipelines, banded rendering | **passes** |

## Environment

- `rustc 1.95.0-nightly (95e5bda86 2026-04-15) (1.95.0.0)` and esp 1.96.0.0, installed via espup
- macOS host (aarch64-apple-darwin), ESP-IDF v5.3.3, `ESP_IDF_TOOLS_INSTALL_DIR=global`
- Target: `xtensa-esp32s3-espidf`, `-Zbuild-std=std,panic_abort`, linker `ldproxy`, `--cfg espidf_time64`
- Chip: ESP32-S3 rev v0.2 (also reproduces on QEMU `-machine esp32s3`)

I'm happy to bisect further (e.g. `#[inline(never)]` probing of `vello_common::strip`/`flatten`) if that helps pinpoint the miscompiled function.
