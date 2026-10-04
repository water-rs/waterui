# cherenkov-bench

Engine adapters and the render/measure CLI for the Cherenkov cross-engine
suite. Adapters are feature-gated: `vello-classic`, `vello-hybrid`,
`vello-cpu`, `skia`, `skia-metal`.

The scene trees every command below reads (`scenes/corpus`, `scenes/perf`)
and the fonts they shape with (`scenes/fonts`) are generated, not
committed. Produce them once per checkout with
`python3 scenes/tools/generate.py` (its Python steps need
`python3 -m pip install -r scenes/fonts/tools/requirements.txt`).

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

A frame's seconds are `encode + submit`, raised to the frame's GPU time
where the backend reports it. The per-scene report records each engine's
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
