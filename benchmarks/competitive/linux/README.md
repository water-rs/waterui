# Competitive benchmark — Linux

Linux leg of the competitive benchmark defined in
[water-rs/waterui#1262](https://github.com/water-rs/waterui/issues/1262):
WaterUI (hydrolysis) vs Flutter, Electron, GTK 4, measured identically on
workloads W1–W4.

## Layout

| path | what |
|---|---|
| `manifest.toml` | every framework/toolchain version pin; the runner reads it |
| `docker/Dockerfile` | the single build+run image (debian:trixie, Mesa 25 llvmpipe+lavapipe, wlroots 0.18, GTK 4.18, Flutter 3.47.5, Electron runtime deps, rustc 1.98.1) |
| `benchcomp/benchcomp.c` | headless wlroots compositor: per-surface present timestamps (presentation-time feedback), scripted pointer input, cgroup v2 memory sampling, spawn→first-frame timing, JSONL event log |
| `gtk4/` | the GTK 4 native contestant (W1–W5 selected by `BENCH_WORKLOAD`); the shared cross-platform contestants live under `../apps/` |
| `runner/runner.py` | single entry point (`uv run`) — builds, stages, generates per-workload drive scripts from the manifest's `[fling]` declaration (wheel-axis scroll for W2/W4), runs, aggregates into one JSON |
| `runner/gpu_probe.py` | in-container adapter probe — parses `vulkaninfo --summary` (pinned image's vulkan-tools) and joins adapters to /dev/dri render nodes by sysfs vendor/device id; `--self-test` parses committed fixtures |
| `runner/report.py` | renders the JSON into markdown tables |

## Method

Every contestant runs under **the same compositor** (`benchcomp`, a headless
wlroots compositor inside the shared container image) with the same Wayland
socket, the same output size (1280x800 @ 60Hz vsync timer), the same pointer
input script, and the same cgroup-based memory accounting. Frame timing comes
from compositor-level present events (`wp_presentation` feedback timestamps,
CLOCK_MONOTONIC), identical for every contestant — no per-app instrumentation.

**Publishable frame-time and memory numbers require a hardware GPU host.**
On a host with exactly one, the runner mounts only that adapter's `/dev/dri`
render nodes into the container and sets `MESA_VK_DEVICE_SELECT` + `DRI_PRIME`
so every contestant renders on it; with several hardware adapters `--gpu
<name>` picks one (the runner refuses otherwise — enumerating a GPU does not
prove a contestant rendered on it). Without a hardware adapter the
container falls back to Mesa's software stack (llvmpipe GL + lavapipe
Vulkan) and the runner refuses to measure — a software-rasterizer number
is development data, not evidence. `--development` overrides the refusal
and marks the emitted JSON/report development-only. For the WaterUI
contestant the runner additionally verifies the adapter hydrolysis itself
logs (`RUST_LOG=hydrolysis::gpu=info`, `selected wgpu adapter ...`); a run
that actually used a CPU-type adapter is refused without `--development`.

## Metrics (per contestant × workload, ≥5 reps, median/min/max + samples)

- package size of the installed directory, uncompressed and gzip
- cold launch → first committed present
- memory steady-state and peak: median and maximum of the cgroup's
  `memory.current`, sampled every 100 ms inside the measurement window
- renderer evidence: the DRM fdinfo usage counters (`drm-engine-*`,
  `drm-cycles-*` on xe) of every render-node fd the contestant's processes
  hold, read by benchcomp once at window start and once at window end. A
  hardware rep must show GPU work on the selected adapter's render node
  across the window; a software rep must show none on any node, with a
  software rasterizer mapped. The driver libraries mapped and nodes held
  at window end are recorded as supporting evidence only (the Vulkan
  loader maps every installed ICD). A driver that keeps no fdinfo usage
  counters cannot prove hardware rendering, and any evidence read failure
  fails the rep
- frame p50/p90/p99, dropped %, fps — W2 and W3 only, per the issue

## Accounting and output

Every attempt is recorded: each rep writes
`results/bench-<name>-<workload>-<rep>-<run_id>.jsonl` through a
flock-protected run (a second concurrent `runner.py` exits with an
error). Failed attempts are kept as error records in the results JSON
(`runs_attempted`, `runs_succeeded`, `failures`); statistics cover only
successful reps. A cell that reaches fewer than the declared
repetitions (default 5) writes `results-INCOMPLETE-<ts>.json` and the
runner exits 2 — a short measurement is never reported as complete. Each
container is launched as `bench-linux-<run_id>-<n>`, removed after its
own rep (and on any abnormal exit), and scoped to owned run state only.

## Run

```sh
uv run runner/runner.py            # full: build image + contestants, 5 reps
uv run runner/runner.py --skip-build
uv run runner/runner.py --development   # allow a software GPU adapter; output labelled development-only
uv run runner/runner.py --gpu "RX"      # required on a multi-hardware-adapter host
uv run runner/gpu_probe.py --self-test  # fixture parse check (no GPU needed)
uv run runner/report.py results/results-<ts>.json -o report.md
# add a per-cell Δ vs an earlier run:
uv run runner/report.py results/results-<ts>.json \
    --baseline results/results-<earlier-ts>.json -o report.md
```

Prereqs on the host: `docker` (privileged containers for cgroup v2), `uv`,
and `cargo` — the runner provisions the `water` CLI itself from THIS
checkout (`cli/` is a workspace member; `cargo install --locked --path cli`
into the suite-shared `benchmarks/competitive/.cache`, serialized by a file
lock), and the checkout HEAD sha is recorded as the framework+CLI+backend
identity.

The WaterUI contestant is the shared `../apps/waterui` project — a
workspace member carrying only `Cargo.toml`, `Water.toml`, `src/` and
`assets/`; `water package` materialises `backends/` at build time, and it
is gitignored, not committed.

## Upstream issues found while building this

Filed upstream; this tree carries no patches or workarounds for them.

1. **water-rs/cli#203 — dev-channel dependency check + stale lock seed**:
   `water build` on a fresh `water create` project against the dev channel
   flags legitimately-resolved sibling packages as Water.lock conflicts
   (`src/project_model/framework.rs`), and the scaffolded backend's
   `Cargo.lock.seed` pins an accesskit generation that no longer resolves.
   Fixed upstream by cli#206 — the runner provisions the `water` CLI
   from this checkout, which postdates that merge.
2. **water-rs/hydrolysis#233 — `scroll()` ignored wheel input**: fixed by
   hydrolysis#234 (`ba6db6b`). Scrolling is now driven from outside the
   app by the compositor's virtual pointer (WORKLOADS.md fling program),
   so the fixed wheel-input path is what the drive actually exercises.
3. **Harness fix (r3)** — `benchcomp` passed the wheel detent count as
   `wlr_seat_pointer_notify_axis`'s `value_discrete`, but wlroots reads that
   argument in value120 units (`WLR_POINTER_AXIS_DISCRETE_STEP` = 120,
   `types/seat/wlr_seat_pointer.c` ~L351): for clients whose `wl_seat` is
   older than v8 it accumulates units until they reach one detent, so a raw
   count of 1 delivered *no* wheel events to them at all. Only
   waterui-hydrolysis binds wl_seat v7 (winit's maximum); GTK4/Electron/
   Flutter bind ≥v8 and got `axis_value120` + a continuous `axis` delta, so
   they scrolled. Fixed in `benchcomp/benchcomp.c` — the compositor now emits
   `a * 120` per detent. The fix predates neither contestant: it corrects the
   emitted protocol events identically for all clients.
4. **Hydrolysis on software Vulkan**: adapter selection rejects CPU-type
   devices; `WATER_HYDROLYSIS_FORCE_FALLBACK_ADAPTER=1` (documented escape
   hatch, `platform.rs` ~L570) is required for lavapipe. The manifest records
   the flag so re-runs are identical.

## Limitations on a GPU-less host

- No GPU → the guard refuses to measure; with `--development` everything
  runs on llvmpipe/lavapipe and frame times reflect CPU rasterisation for
  every contestant equally — development numbers only.
- No DRM device → benchcomp advertises the dmabuf global through a fabricated
  `drmDevice` (interposed `drmGetDeviceFromDevId`); with the pixman renderer
  the compositor still timestamps and feeds back every commit/present.
- Electron requires `--no-sandbox` inside containers (kernel userns rules).
- **Electron W3 only**: Chromium's viz frame submission stalls for continuous
  rAF animation under this headless wlroots compositor — rAF ticks at 60fps
  and `document.visibilityState` is `visible`, but `wl_surface.commit` is
  only invoked a handful of times per run (identical with `--disable-gpu` and
  SwiftShader; pointer frames, output enter/leave, seat keyboard focus and a
  non-zero present refresh were all ruled out). Input-driven frames — W2/W4
  scroll — submit normally at 60fps. The W3 frame metrics reported for
  Electron reflect only the frames it actually committed; the limitation is
  recorded in the results JSON and the report.
- The wlroots headless backend emits `wp_presentation_feedback.presented`
  with `refresh=0`/`seq=0`; frame timing here is compositor-side (the
  `present` events logged per output frame), so client-side presentation
  feedback values are not used.
