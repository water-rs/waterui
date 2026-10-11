# #158 validation: the gamut map on real devices

#96 chose Ottosson's analytic OKLab clip over the CSS Color 4 chroma
binary search on lavapipe timings. #158 re-measured on real hardware:
variant A is dev's adaptive analytic map (`present.wgsl`, commit
773f9f58's map family, including the #98 `dest` generalisation); variant
B replaces `gamut_map` with the spec's OKLCh chroma binary search
(local-MINDE JND 0.02, epsilon 1e-4, `dest`-parametric like the rest of
the shader). Both were run through `cherenkov-bench present-cost`
(pass-boundary GPU timestamps, one submit per frame, `Rgba8Unorm`
destination, `srgb-hw` present) at native resolution on each device, in
interleaved rounds A,B,A,B…, 7 rounds per variant per pattern, 120
measured frames each. Every device interaction ran under the shared
`/tmp/device-locks` fcntl lock; every iOS round's `done.json` carries the
launch's `run_id`, proving the pulled report came from the binary the
round installed.

## Correctness (M2 Max, Metal)

The f64 sweep reproduces #96's report exactly (adaptive vs the CSS spec
map: ΔE_OK mean 0.0104, p99 0.072, max 0.177; hue p99 5.35°, max 7.5°).
The GPU harness (`bench/examples/gamut_gpu_sweep.rs`, 29,954
out-of-gamut samples through `present.wgsl`) tracks each map's f64
reference to the unorm8 floor:

- variant A (adaptive WGSL): ΔE_OK vs adaptive f64 mean 0.00076, p99
  0.0017, max 0.013 (≤1 LSB); vs CSS f64 mean 0.0096, p99 0.047,
  max 0.176 — the algorithmic distance between the maps.
- variant B (CSS WGSL): ΔE_OK vs CSS f64 mean 0.00067, p99 0.0015, max
  0.037 (≤2 LSB); vs adaptive f64 mean 0.0096, p99 0.047, max 0.184.

The spec map's *returned* bytes still shift hue (its local-MINDE rule
returns a channel-clipped colour near the hue ray): the f64 CSS map
itself measures hue p50 1.44°, p99 8.2° vs input, against the adaptive
map's 0° p50 / 5.3° p99 / 7.5° max. On P3 primaries the two maps land
~0.021–0.026 ΔE_OK apart.

## iPhone 16 Pro (A18 Pro, Metal, 1206×2622, iOS 27)

| pattern | map | p50 median (ms) | p50 spread | p99 worst (ms) | of 8.33 ms |
|---|---|---:|---:|---:|---:|
| oog | adaptive | 11.295 | 10.228–11.800 (13.9%) | 12.704 | 136–153% |
| oog | css | 28.109 | 28.067–28.140 (0.3%) | 31.786 | 337–382% |
| mixed | adaptive | 5.791 | 5.771–5.800 (0.5%) | 6.044 | 70–73% |
| mixed | css | 12.111 | 12.101–12.130 (0.2%) | 13.165 | 145–158% |

CSS/adaptive p50: 2.49× on oog, 2.09× on mixed. Thermal state (from the
app, `ProcessInfo.thermalState`): nominal (0) on 26 of 28 runs, fair (1)
on the first round's two.

## Mac mini M1 (Metal, 1920×1080)

| pattern | map | p50 median (ms) | p50 spread | p99 worst (ms) | of 8.33 ms |
|---|---|---:|---:|---:|---:|
| oog | adaptive | 8.237 | 8.212–8.244 (0.4%) | 10.852 | 99–130% |
| oog | css | 24.324 | 24.313–24.393 (0.3%) | 24.691 | 292–296% |
| mixed | adaptive | 4.747 | 4.710–4.759 (1.0%) | 5.241 | 57–63% |
| mixed | css | 10.391 | 10.354–10.411 (0.6%) | 13.533 | 125–162% |

CSS/adaptive p50: 2.95× on oog, 2.19× on mixed. `pmset -g therm`
reported no thermal warning before or after every run.

## Decision

The binary search is decisively outside round-to-round noise on both
devices — 2–3× slower per pattern, and on every-OOG-pixel input the
CSS present pass alone costs 3–4× the 120 fps frame budget. The
adaptive analytic map stays; nothing in the engine changes. The
lavapipe ~3× ratio #96 measured generalises to real Apple GPUs.

Caveat: both patterns are synthetics that gamut-map far more pixels
than a real frame (`oog` maps every pixel of a 2–3 MP texture); even
variant A exceeds 8.33 ms there. The comparison isolates the map's
marginal cost — roughly +17 ms/frame (iPhone) and +16 ms/frame (M1)
for CSS on all-OOG input, proportional to OOG pixel count.

The iOS host needed UIScene lifecycle adoption (iOS 27 traps at scene
creation otherwise, landed on dev separately) and links cherenkov-gpu's
CoreMedia/CoreVideo/AVFoundation deps; `BenchRunner` also gained a
`run_id` in `done.json` plus per-run thermal state, both committed here.
