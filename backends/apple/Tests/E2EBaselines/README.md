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
black and white and retains the larger per-pixel error. The separate maximum
local error uses the oracle's 3x3 box-filtered premultiplied RGBA comparison.

All three limits must pass. The shared defaults are `flip_mean: 0.006`,
`flip_max: 0.15` and `max_local_error: 0.09`, from the oracle's constants.
Frame settling and stored-baseline comparisons always use these defaults.
Only SwiftUI parity reads reviewed overrides from `parity-budgets.json`, or
from the file selected by `FLIP_TOLERANCES`.

An override is an object containing all three fields, grouped by platform and
example. An `ios@27` entry takes precedence over `ios`; missing entries use
the shared defaults. The file currently contains no exceptions. Every entry
is validated, including unused entries. Old changed-pixel fractions are
rejected rather than reinterpreted as FLIP limits. There are no bottom-band
masks or automatic platform-flake exemptions.

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

Use the existing captures as inputs:

```sh
cargo run --locked -p cherenkov-oracle --features apple-screenshots \
  --bin compare-screenshots -- compare \
  ios-reminders-ref.png ios-reminders.png ios-reminders-heatmap.png \
  --platform ios --example reminders
```

Shared-default comparisons may omit an unknown OS version; JSON records
`os_major: null`. Supplying `--tolerances` requires an explicit `--os-major`
so versioned overrides cannot be selected from guessed metadata. CI derives
that version from the running platform.

Gate mode exits 0 within limits, 1 outside them, and 2 for operational errors.
`--record` changes a completed comparison's exit status, not its limits or
reported acceptance. `check IMAGE` validates PNG decoding only; a legal
uniform image is not rejected as blank. Every result still requires visual
review.
