# cherenkov-bench

Engine adapters and the render/measure CLI for the Cherenkov cross-engine
suite. Adapters are feature-gated: `vello-classic`, `vello-hybrid`,
`vello-cpu`, `skia`, `skia-metal`.

The scene trees every command below reads (`scenes/corpus`, `scenes/perf`)
and the fonts they shape with (`scenes/fonts`) are generated, not
committed. Produce them once per checkout with
`python3 scenes/tools/generate.py` (its Python steps need
`python3 -m pip install -r scenes/fonts/tools/requirements.txt`).

## Reading a measure report

`measure` renders offscreen: no frame reaches a compositor, so there is no
present-to-present time. Each entry of `samples[]` is one frame's raw
timing, and `percentiles` gives p50/p90/p99 of each field:

- `encode_seconds`, `submit_seconds`, `gpu_wait_seconds` — host wall time
  of encoding, of submission (rasterization included for CPU engines), and
  of explicit GPU waits inside submission, which `submit_seconds` excludes.
- `gpu_seconds`, `passes`, `phases` — GPU time from real timestamps, per
  pass, and per render-thread phase; `null` or omitted where the backend
  provides none, with `gpu_timing_unavailable` naming why. Never estimated.
- `latency_seconds` — encode-to-completion wall time, present only under
  `--wait-for-completion`, which finishes each frame before the next
  encode.
- `completed_seconds`, `completion_interval_seconds` — when the frame's
  rendering completed, in host seconds since the measured window opened,
  and the interval since the previous measured frame completed (`null` on
  the first). The window opens on an idle GPU, so no warmup frame
  completes inside it. The interval is the measured frame time. Its percentiles are
  `percentiles.completion_interval_seconds`. Both fields are `null` when the
  adapter has no completion signal, and `completion_unavailable` says why.

By default frames are submitted asynchronously, so one frame's encode can
overlap an earlier frame's GPU work. The stage fields therefore do not add
up to a frame time, and the report never derives one from them. The
pipelined frame time is `completion_interval_seconds`, measured from each
adapter's own completion signal, which `completion_signal` names.

Measuring completion leaves every timed interval as it would be without
it. The frame loop arms each frame's signal in a separate adapter step
that runs after `submit_seconds` (and, under `--wait-for-completion`,
`latency_seconds`) has closed. Only a Skia Vulkan semaphore has to be
attached inside submission, because Ganesh signals it with the frame's
flush; creating and exporting it happen in that step. The signals:

- CPU engines complete when their raster work ends inside submission.
- The Cherenkov GPU and Vello hybrid adapters use wgpu's
  `on_submitted_work_done`, registered once the frame's submit returns.
  wgpu runs that callback only inside a later device poll or queue
  submission, so the instant is an upper bound on completion. A GPU-bound
  run sees completion while the engine waits on the device. A CPU-bound
  run sees it when the next frame submits, so its intervals follow the
  submission cadence.
- Vello classic drains the GPU inside every submission to verify its
  overflow feedback. Its frames never overlap, so the interval is a
  serialized frame's cost plus the loop's overhead, not pipelined
  throughput.
- Skia Metal commits one empty command buffer on Graphite's queue once
  each submit returns. The instant is when Metal ran that buffer's completed
  handler. Metal completes a queue's buffers in commit order, so the
  instant bounds the frame's completion from above. The marker costs one
  empty commit per frame.
- Skia Vulkan gives Ganesh a binary semaphore, created before the frame,
  to signal with the frame's flush, then exports it as a Linux sync file.
  The instant is the kernel's signal timestamp for that fence, read after
  the run drains. A sync file the driver exports already signaled carries
  no time, and the instant is then the export's return, an upper bound. A
  driver that cannot export sync files reports `completion_unavailable`
  instead.

Under `--wait-for-completion` the interval is also measured, but each
frame then waits for the one before it, so the interval tracks
`latency_seconds`. The signal is armed only after that wait returns, so
`latency_seconds` keeps its interval. The wgpu callback then runs in the
per-frame drain that follows, and the Skia Metal marker is committed
after the frame finished, so both instants fall after the wait's end. A
Skia Vulkan sync file keeps its fence time unless the driver exports it
already signaled, when the instant is again the export's return. The CPU
engines and Vello classic record theirs inside submission, unaffected.
Whether an engine keeps a frame rate is `--rate`'s
`pacing` block (`achieved_hz`, `missed_deadlines`), and energy per frame
is `--energy`, both described below.

## Native resolution

`measure --native WxH` renders into a `W`×`H` surface — the device's own
resolution, which a windowed host would read from its window and an
offscreen run supplies on the command line — with the scene drawn under
the uniform scale `s = W / scene_width`, like a device pixel ratio, so
the pixel load matches the device rather than the scene file. The
report's `native` block records the size and `s`; the top-level
`width`/`height` are the rendered surface size. Pair with `--rate` to
pace at the panel's refresh:

    cherenkov-bench measure --engine cherenkov --scene scenes/perf/chart \
        --native 2752x2064 --rate 120 --frames 120 --cpu 7 \
        --out measure-cherenkov-chart-native.json   # iPad Pro M4

## Capacity sweep

`capacity` finds the largest scene load each engine sustains within a
frame budget. The draw list is repeated `k` times — each copy offset by
a fixed translation that wraps inside the canvas, so element count and
overdraw grow linearly — doubling `k` until a probe's p99 frame time
exceeds `--budget-ms` (default 8.333, the 120 fps budget), then
binary-searching the largest `k` that stays within it. Each probe is an
interleaved round across the `--engine` list at `measure`'s warmup and
frame counts:

    cherenkov-bench capacity --engine cherenkov --engine vello-classic \
        --corpus scenes/perf --frames 60 --warmup 5 --cpu 7 \
        --out-dir capacity-out

Every probe runs with `--wait-for-completion`, so a frame's seconds are
its `latency_seconds`: synchronous encode-to-completion time, not a
pipelined-throughput estimate from stage times. The per-scene report records each engine's
max sustained `k`, the p99 at `k` and at `k + 1`, and every probe.
`--max-k` (default 1024) caps the doubling; `--native` retargets every
probe exactly as for `measure`.

## Measuring on big.LITTLE hardware

On hosts whose CPUs have differing `cpuinfo_max_freq` (e.g. Tensor G4:
four 1.95 GHz cores, three 2.6 GHz, one 3.1 GHz) an unpinned `measure`
run lands on any cluster — the same scene can time 10x apart depending
on which core the scheduler picked. Pass `--cpu` to pin the measuring
thread (and every thread the adapter spawns afterwards) to one set:

    cherenkov-bench measure --engine vello-cpu --scene scenes/perf/chart \
        --frames 60 --cpu 7 --out measure.json        # big core only
    cherenkov-bench measure ... --cpu 4-6              # mid cores
    cherenkov-bench measure ... --cpu 0-3              # little cores

`--cpu` works on Linux and Android; elsewhere it is refused with an
error. Each sample records the CPU it started and ended on
(`sched_getcpu`); a sample that migrated mid-phase is flagged and the
report counts them. The report's `placement` block shows the requested
set, `controlled`/`heterogeneous` flags, per-CPU sample counts and each
CPU's `cpuinfo_max_freq`. Without `--cpu` on a heterogeneous host the
run still proceeds but the report marks `controlled: false` and the log
warns once — publish those numbers only with the caveat attached.

## Measuring energy per frame

`measure --rate <hz>` paces the measured frames at a fixed rate instead
of running flat out: frame `n` starts on the deadline `start + n/rate`
(the one sleep in the loop is frame pacing). The report's `pacing`
block records the requested rate, the achieved rate, and how many
frames missed their deadline. Energy per frame is only comparable
between engines at a fixed rate.

`measure --energy` brackets the measured window — after warmup, from
just before the first measured frame to just after the last — with the
platform's power meter and adds an `energy` block: joules, joules per
frame and average watts per rail and in total.

**Android** (Pixel-class devices with ODPM): every rail named in
`/sys/bus/iio/devices/iio:device*/enabled_rails` is read from
`energy_value` before and after the window. The rails are root-only —
run the bench rooted:

    adb shell su -c '/data/local/tmp/cherenkov-bench measure \
        --engine vello-cpu --scene /data/local/tmp/scenes/perf/chart \
        --frames 120 --rate 60 --cpu 7 --energy --out /data/local/tmp/measure.json'

`--energy` is mandatory: if the rails can't be read, the command fails
with an error naming the path and the permission rather than writing a
report without energy.

**macOS**: `sudo -n powermetrics --samplers cpu_power,gpu_power -i <ms>
--format plist` runs for exactly the measured window and the CPU, GPU
and ANE package energies are summed. `sudo -n` must be permitted (a
cached credential or a sudoers entry); if it isn't, the command fails
with a clear error.

The report's `conditions` block records what else skewed the numbers:
thermal status (`dumpsys thermalservice` severity on Android,
`powermetrics` `thermal_pressure` on macOS) and the first readable
`/sys/class/thermal` zone temperature, screen state and brightness
where readable, alongside the `placement` block `--cpu` already
produces. Screen brightness is not recorded on macOS or Windows.

## Comparing two builds on the Pixel

GPU time scales with the GPU clock, and the default governor picks the clock
per process from recent load: identical builds measured back to back land on
different operating points and differ by the clock ratio. On Android `measure`
records the GPU's clock residency over the measured window (`gpu_clock`, from
the Mali driver's `time_in_state`), so every report shows the clocks its GPU
time ran at.

`scripts/pixel_compare.py run` compares two Android builds over interleaved
base/head rounds with the GPU and CPU 7 pinned to one clock each
(`--gpu-khz`, `--cpu-khz`; 649 MHz and 2.294 GHz by default), restoring the
original limits afterwards. Every invocation's controls are checked and
recorded — screen off, thermal status, cooling devices, the pinned policy
ranges, GPU and CPU residency, CPU placement — and a failed control stops the
collection. `scripts/pixel_compare.py analyze OUT` reports, per scene, the
median of each arm's per-invocation GPU p50, the change, and its 95% interval
from a hierarchical bootstrap over invocations and their frames. A scene whose
whole interval lies above +2% fails at any width. Any other scene is decided
only when its interval is no wider than the 2% limit: it fails above +2% and
passes otherwise. A wider interval is `unresolved`, never a pass; that holds
for an interval wholly below −2% too, since the limit bounds a cost increase
and a decrease is no failure. Full-size scenes resolve in 120
frames per invocation. `--governor` leaves the clocks to the governors, for
the 8.33 ms p99 budget check:

    bench/scripts/pixel_compare.py run --serial pixel-9-pro:5555 \
        --base bench-dev --head bench-change \
        --scene scenes/perf/map --scene scenes/perf/chart --out compare
    bench/scripts/pixel_compare.py analyze compare

### Sub-millisecond scenes: shader-core cycles

A scene whose GPU time is a fraction of a millisecond is not judged by GPU
time on the Pixel (#2775). Its frame is latency-bound: a fixed buffer clear
added just before the pass moved continuous-s50 by −2.5% at
`--frames 1200 --rounds 16`, because added work is absorbed or changes the
GPU's power state. Such scenes are compared by cost instead:
`measure --gpu-counters` reads the Mali hardware counters through the kbase
driver (root), waits for the GPU to go idle after every measured frame and
samples, so each frame records exactly the counters its own work added —
shader-core active cycles summed over the cores, with fragment, non-fragment,
GPU-active and tiler cycles and fragment warps beside them. The wait changes
the run's GPU timings, so a counter run is never read for time. The counter
layout read is the Mali-G715's; another GPU fails rather than read a
different layout under these names.

`pixel_compare.py run --gpu-counters` compares the shader-core active cycles
under the same interval rule. Every process settles at its own cycle level,
a few percent apart for identical binaries, while frames inside one agree to
about 1%, so these scenes need many short invocations:

    bench/scripts/pixel_compare.py run --serial pixel-9-pro:5555 --gpu-counters \
        --rounds 32 --frames 300 --base bench-dev --head bench-change \
        --scene scenes/corpus/continuous-s50 \
        --scene scenes/corpus/stroke-join-continuous-s00-miter-lo --out compare-cycles

At those settings an identical-binary comparison passes both 96 px scenes
(intervals within ±1.1%). A 4 KiB buffer clear added to every pass fails them
at +5.1% and +4.7%, and the same clear adds the same cycles to chart, so added
work reads as the same cost in every scene.

## Sparse live updates

`scenes/perf/live-dashboard` changes one two-digit glyph run and one bar height
per frame. Static page content is recorded once. To compare retained lowering
against a saved baseline executable on Linux/lavapipe, run both binaries with
identical affinity and frame counts:

    VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json \
      cherenkov-bench measure --engine cherenkov \
      --scene scenes/perf/live-dashboard --warmup 10 --frames 60 \
      --cpu 0-3 --out gpu-live-dashboard.json
    cherenkov-bench measure --engine cherenkov-cpu \
      --scene scenes/perf/live-dashboard --warmup 10 --frames 60 \
      --cpu 0-3 --out cpu-live-dashboard.json

Use the ICD path and allowed CPU set reported by the measurement host. Alternate
before/after runs to expose host variation. Compare `encode` and `submit`
separately: only `submit` contains the render-thread lowering being optimized.
