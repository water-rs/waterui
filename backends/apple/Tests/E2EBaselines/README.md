# E2E screenshot baselines

The Apple E2E suites capture WaterUI examples on iOS and macOS. Registered
SwiftUI twins are captured on the same runner. Original captures, comparison
JSON and FLIP heatmaps remain available as run artifacts for whole-image
visual review. A passing score does not establish that the UI is correct.

The comparer is the `compare-screenshots` binary in `cherenkov-oracle`, enabled
by `apple-screenshots` on ARM64 macOS. Image I/O and CoreGraphics decode PNGs
into premultiplied extended linear Display P3. Wide gamut, HDR and 16-bit
precision are preserved. Invalid PNG metadata, unsupported ICC profiles,
incomplete images and mismatched dimensions are errors, not visual differences.

## Comparison limits

Comparisons use the oracle's FLIP implementation: LDR-FLIP for SDR inputs and
HDR-FLIP when either input requires it. FLIP compares transparent inputs over
black and white and retains the larger per-pixel error. There are two limit
sets, selected with `--limits`; there are no per-example overrides.

`route` (the default) judges two routes to the same pixels: frame settling and
stored-baseline comparisons. It uses the oracle's constants, `flip_mean:
0.006`, `flip_max: 0.15` and `max_local_error: 0.09` (the 3x3 box-filtered
premultiplied RGBA difference), and all three must pass.

`twin` judges a WaterUI screen against its SwiftUI twin, two different
implementations. Edge offsets of up to 2 px are not drift; missing, extra or
clearly misplaced elements are. It uses FLIP alone, after alignment: the
actual image is displaced by every whole-pixel offset within ±2 px on each
axis (25 offsets) and compared over the overlap.

- `flip_mean`, the mean of the FLIP map at the offset with the lowest mean,
  must be at most 0.018.
- `max_block_flip_mean` must be at most 0.36. For every square window of
  8 pt, at every position in the reference frame, take the lowest mean FLIP
  error over the offsets whose overlap contains it, since two layouts can
  offset different regions in different directions; the score is the
  largest of these. The window is `8 × --scale` pixels: the noise two text
  stacks leave on one glyph run grows with the display scale, so a window in
  pixels cannot separate the 3x iOS captures from the 1x macOS ones.

`--limits twin` requires `--scale`, the captures' pixels per point. CI
derives it from the simulated device's screen scale on iOS and from the
runner's display on macOS.

The limits were calibrated on nightly captures of every registered twin,
19 pairs equivalent by eye and 21 with visible drift; every pair was reviewed
by eye. The closest pairs on each side:

| Pair | Eye | `flip_mean` | `max_block_flip_mean` | Gate |
| --- | --- | --- | --- | --- |
| ios-hover | equivalent | 0.0120 | 0.316 | pass |
| ios-shape | equivalent | 0.0087 | 0.323 | pass |
| macos-locale | equivalent | 0.0092 | 0.320 | pass |
| macos-edge_text | drift: leading whitespace dropped | 0.0014 | 0.411 | fail |
| ios-picker | drift: wheel rows at another pitch | 0.0119 | 0.416 | fail |
| macos-markdown | drift: inline code chip, scroller | 0.0010 | 0.434 | fail |
| macos-navigation | drift: "Inbox (3)" vs "Inbox" | 0.0053 | 0.448 | fail |
| ios-typography-rtl | drift: mirrored layout | 0.0285 | 0.280 | fail |
| macos-typography-rtl | drift: mirrored layout | 0.0300 | 0.205 | fail |

Equivalent pairs reach 0.0120 and 0.323; the drifted pairs start at 0.0285
on the mean and 0.411 on the block, gaps of 2.4x and 27%. Every other
drifted pair scores at least 0.58 on the block.

## Capture paths

No baseline image is committed here. A stored-baseline comparison runs only
when `<platform>/<example>.png` has been supplied; otherwise the report says
that no baseline is available and requests visual review.

- `<example>.skip` declares a continuously changing first screen. It takes
  one supervised capture after first paint instead of waiting for settling.
  Launch and PNG decoding still gate; comparison is skipped.
- `<example>.visual` keeps the WaterUI capture and registered SwiftUI twin
  for visual review without a numerical parity gate.
- `<example>.parity-skip` skips the twin comparison while a known divergence
  is being resolved.

These markers do not establish visual correctness. Inspect the whole capture,
not only the heatmap or a selected region.

## Recording

Dispatch the Apple E2E suite with `record_baselines: true` to retain captures
and comparison reports. Record mode reports the actual metrics, limits and
`within_tolerance` result even when it exits successfully. Decode and other
operational failures still fail the run. Recording never raises acceptance
limits. The publish job updates release measurements; images and FLIP reports
remain review artifacts, not automatically accepted references.

## Manual comparison

Use the existing captures as inputs; this judges a SwiftUI twin pair:

```sh
cargo run --locked -p cherenkov-oracle --features apple-screenshots \
  --bin compare-screenshots -- compare \
  ios-reminders-ref.png ios-reminders.png ios-reminders-heatmap.png \
  --platform ios --example reminders --limits twin --scale 3
```

Omit `--limits` and `--scale` for the route limits. `--os-major` is recorded
in the JSON when given (`os_major: null` otherwise); CI derives it from the
running platform.

Gate mode exits 0 within limits, 1 outside them, and 2 for operational errors.
`--record` changes a completed comparison's exit status, not its limits or
reported acceptance. `check IMAGE` validates PNG decoding only; a legal
uniform image is not rejected as blank. Every result still requires visual
review.
