# Hydrolysis AGENTS.md

This file carries Hydrolysis's engine contract, referenced from the root
`AGENTS.md`. The repository-wide rules in the root file apply to every file in
this crate too.

## Self-drawn rendering: Hydrolysis on Cherenkov

Hydrolysis is WaterUI's only self-drawn backend (water-rs/hydrolysis#187). It renders through Cherenkov (water-rs/cherenkov), which replaces Vello, and covers two design points that are selected per target at build time. The two are never switched at runtime, and neither is a fallback for the other:

- **GPU (`cherenkov-gpu`)** for phones, tablets and desktops. The performance target is 120fps, an 8.33 ms frame budget at p99, on every scene, because current devices have high-refresh panels. 60fps is accepted only for a scene that is too complex to fit the budget in theory and that no other renderer in the harness fits either; it is never the default target. High refresh is requested **explicitly** per platform (ProMotion, high-refresh display links) and never left to incidental vsync, and there is no hard frame cap. It uses modern GPU compute **and** multi-core CPU: single-threaded execution is a gap to close, not the target. No runtime rendering path reads GPU targets back to the CPU; offscreen snapshot export is a separate verification path.
- **CPU (`cherenkov-cpu`) at the microcontroller design point** for MCU-class devices with no GPU and no full-resolution framebuffer. It uses banded output through a small scratch buffer, native panel formats, command-level damage for partial panel transfers, and power-frugal frame rates (30/60fps). Firmware builds prune features so that `wgpu` and other heavyweight crates never enter the graph. It is `std`-based on an embedded RTOS, not bare-metal `no_std`.
- **Damage is an engine concern.** Partial updates are an engine-internal optimization, verified bit-exact against a full render, and never part of the view-level contract.
- **Hosts.** The embedded host family lives in Hydrolysis next to the windowed hosts: panel flush sinks (RGB565 and others over SPI/QSPI, plus a simulator window), the embedded executor with its per-frame tick, and input routing for touch, encoders and buttons. Dispatch, layout and input handling stay shared through `waterui-backend-core`. water-rs/dew retires once its examples, tests and CI coverage have moved; until then it changes only to carry that migration.
- The frame model is decided by measurement, not by rule. Hydrolysis carries WaterUI's fine-grained reactivity into the engine (water-rs/hydrolysis#205): a paint-only change (colour, opacity, transform, a text run's content) reaches the engine as a live operand of the recorded command with no Hydrolysis tree walk and no layout pass; a layout-affecting change relayouts what it affects, with layout semantics exactly as `docs/layout-spec.md` defines them; animations are sampled where the engine samples them; an idle window does no CPU or GPU work. Today's pump still re-reads every reactive input, relayouts and re-encodes the whole retained tree on every awake frame (`refresh_window_scene` in the renderer's `src/runner/window.rs`) — that is the state being replaced, not a contract to preserve.
- Every such choice is settled by a paired A/B measurement on a real device of three costs together: frame time (CPU per phase, GPU time, p99), energy per frame plus the energy of an idle window, and memory (CPU resident set and GPU allocations). Memory is a first-class cost: CPU and GPU throughput keep growing while memory keeps getting more expensive, so a cache or retained layer is adopted only when its measured win in time or energy pays for its memory. Browser-style stacks of retained caches — per-layer raster tiles, duplicated display lists — are the waste to avoid; when recomputing is fast enough, recompute.

## Hydrolysis rules

- **Unsupported components panic — never stub.** Hydrolysis never draws a stand-in for a component it cannot realize. A view that reaches the backend without a realization — e.g. `Native<MapConfig>` because no `Hook<MapConfig>` (such as `waterui_map_gpu::install`) was installed — is a programmer error and must panic at the earliest point it is seen (measure or node build) with a message naming the missing piece and how to install it. See `unsupported_system_icon`, `unsupported_map`, and `unsupported_webview` in `src/renderer/native_measure.rs` for the pattern. Do not add gradient/color/mock substitutes, silent no-ops, or fallback renderings for unsupported primitives — a stand-in that merely *looks* like the component hides the missing realization from the application author.
- **Measurement practice.** Wall-clock time and energy are measured only on quiet real devices (Apple M1, iPad Pro M4, Pixel 9 Pro), with interleaved A/B rounds and per-round results reported; the iPad's GPU clock is bimodal with device state, so read per-round numbers before medians. A shared cloud VM is not a timing instrument: an identical-binary control drifted by up to ±80% there — on VMs, CPU work is compared by deterministic Callgrind instruction counts, and the VM result never stands in for a device measurement. A comparison names both builds by commit and states what each interval covers.

## Layout

- `src/` — the `hydrolysis` crate: renderer, runner, platform, widgets
- `tests/` — the cross-backend integration suites (semantic + offscreen)
- `benches/` — the frame-pump and measurement benchmarks
- `android/` — the Android host (Gradle project plus the `test-app` Rust crate)
- `bench/` — the bench harnesses, including the Android golden pipeline
- `scripts/` — pinned-source asset generators (fonts, Gradle wrappers, media fixtures)
