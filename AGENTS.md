# AGENTS.md

## Unsupported components panic — never stub

Hydrolysis never draws a stand-in for a component it cannot realize. A view
that reaches the backend without a realization — e.g. `Native<MapConfig>`
because no `Hook<MapConfig>` (such as `waterui_map_gpu::install`) was
installed — is a programmer error and must panic at the earliest point it is
seen (measure or node build) with a message naming the missing piece and how to
install it. See `unsupported_system_icon`, `unsupported_map`, and
`unsupported_webview` in `src/renderer/native_measure.rs` for the pattern.

Do not add gradient/color/mock substitutes, silent no-ops, or fallback
renderings for unsupported primitives — a stand-in that merely *looks* like the
component hides the missing realization from the application author.

## Rendering model

Hydrolysis is WaterUI's only self-drawn backend (#187). It renders through
Cherenkov (water-rs/cherenkov), with the backend chosen per target at build
time: `cherenkov-gpu` for phones, tablets and desktops, and `cherenkov-cpu` at
the microcontroller design point. There is never a runtime fallback from one to
the other. The embedded host family lives here next to the windowed hosts:
panel flush sinks, the embedded executor with its per-frame tick, and input
routing for touch, encoders and buttons. water-rs/dew retires once its
coverage has moved.

Self-drawn components render as Cherenkov content inside the window's layer
tree, composited with the rest of the UI in the same frame. A component never
owns a second renderer or a second surface. A map is recorded content: frozen
tile `Picture`s, with the camera as the layer transform (water-rs/map-gpu#46).
A missing engine capability is an engine issue in water-rs/cherenkov, never a
local rasterizer, cache or stand-in here.

## Frame model: measured, not assumed

The frame model is decided by measurement, not by rule (#205). The earlier rule
that every awake frame re-reads every reactive input, relayouts and re-encodes
the whole retained tree (`refresh_window_scene`) is withdrawn. It describes the
state being replaced.

- **Fine-grained reactivity end to end.** nami says exactly which value changed
  and where, and the view tree is almost entirely static: `Dynamic` subtrees are
  the rare exception. Static structure is recorded once and reused, and only the
  operands a signal touches change.
  - A paint-only change (colour, opacity, transform, a text run's content)
    reaches the engine as a live operand of the recorded command, with no
    Hydrolysis tree walk and no layout pass.
  - A layout-affecting change relayouts what it affects. Layout semantics stay
    exactly as WaterUI's `docs/layout-spec.md` defines them; only the
    scheduling changes.
  - A `Dynamic` rebuild stays scoped to its own subtree.
- **The engine drives time.** Animations and scrolling are sampled where the
  engine samples them. The window pump follows the engine's `Next`, and an idle
  window does no CPU or GPU work.
- **Three costs, measured together.** Every frame-model choice is settled by a
  paired A/B measurement on a real device of frame time (CPU per phase, GPU
  time, p99), energy per frame plus the energy of an idle window, and memory
  (CPU resident set and GPU allocations). Prior art is evidence, not doctrine.
  GPU UI renderers that began with whole-frame redraw kept it only because
  their per-frame cost was tiny and everything expensive was cached by content,
  and a browser GPU renderer walked full redraw back to partial present for
  energy.
- **Memory is a first-class cost.** CPU and GPU throughput keep growing while
  memory keeps getting more expensive, so a cache or retained layer is adopted
  only when its measured win in time or energy pays for its memory.
  Browser-style stacks of retained caches (per-layer raster tiles, duplicated
  display lists) are the waste to avoid. When recomputing is fast enough,
  recompute. Content-keyed caches of expensive intermediates (glyph atlas, path
  coverage, blur results, decoded images) are kept where the measurement
  justifies them.
- **120fps is the target.** The budget is 8.33 ms at p99 on every scene,
  because current devices have high-refresh panels. 60fps is accepted only for
  a scene that is too complex to fit the budget in theory and that no other
  renderer in the harness fits either. Never hard-code 60 Hz: use
  `TARGET_FRAME_INTERVAL` wherever the display rate is unknown. Request high
  refresh explicitly per platform, and let slow animations request only the
  rate they need on variable-refresh panels.
- **Energy levers, in expected order of impact:**
  1. no frame while idle;
  2. low per-frame cost;
  3. frame rate on demand;
  4. less memory bandwidth on tile-based GPUs;
  5. system layers and partial present, which are engine concerns.

## Measurement practice

- Wall-clock time and energy are measured only on quiet real devices (Apple M1,
  iPad Pro M4, Pixel 9 Pro), with interleaved A/B rounds and per-round results
  reported. The iPad's GPU clock is bimodal with device state, so read
  per-round numbers before medians.
- A shared cloud VM is not a timing instrument: an identical-binary control
  drifted by up to ±80% there. On VMs, CPU work is compared by deterministic
  Callgrind instruction counts, and the VM result never stands in for a device
  measurement.
- A comparison names both builds by commit and states what each interval
  covers.
