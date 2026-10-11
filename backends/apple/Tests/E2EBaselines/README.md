# E2E screenshot baselines

Each e2e run (`apple-e2e.yml`, invoked by the repository's Nightly) builds
and launches every runnable example on an iOS simulator and on macOS,
captures the first settled screen, and verifies it: non-blank always, and —
where a baseline exists — compared against the recorded reference capture
with the shared comparator's per-channel tolerance and diff budget. No
baseline image is committed to this repository: generated binary captures do
not belong in source history. A reviewed capture becomes the comparison
input through the pinned-generator flow (a record run pins the generator's
source commit, runtime and inputs, and the reference is regenerated at
comparison time); until that lands, the `<platform>/<example>.png` baselines
simply do not exist and the pixel step reports "no baseline yet" rather than
pretending coverage.

- `ios/` and `macos/` — marker files only, never captures:
- `<example>.skip` — an empty marker that skips pixel comparison for an
  example whose first screen cannot be made deterministic (continuous
  animation, live media). The launch and non-blank checks still apply.
- `<example>.visual` — the semantic class for output that is not
  pixel-stable by construction (GPU-rendered, e.g. `filter`). Never
  pixel-compared: the WaterUI capture — and the SwiftUI twin capture where a
  twin is registered — ship as artifacts for human review, and the report
  prints `VISUAL REVIEW REQUIRED`. Launch and non-blank checks still gate.
- `<example>.parity-skip` — an empty marker that skips the SwiftUI parity
  comparison for an example with a registered twin while a known divergence
  is worked down.

## SwiftUI parity

Examples with a twin registered in `backends/apple/Tests/E2EReference` are
additionally compared against the twin rendered live by the reference host
on the same runner — the backend must stay pixel-faithful to what SwiftUI
produces for the same layout. Parity compares a bounded *fraction* of
differing pixels, never an exact-pixel requirement, so GPU-rendered examples
stay on the visual path. `parity-budgets.json`
holds the allowed diff fraction per platform per twin; twins absent from the
file are held to the strict default (2%). Recorded budgets are the measured
divergence plus headroom — raise them only alongside a filed issue for the
divergence they bless.

## Recording

Baselines are recorded on CI, not locally: window metrics, scale factors, and
OS rendering all differ across machines, so a locally recorded baseline fails
on the runner. Dispatch the `Apple E2E Suite` workflow with
`record_baselines: true`; the run uploads its captures as `e2e-baselines-*`
artifacts for review and the `publish-baselines` job merges the text
manifests (parity budgets, the package-size measurement) into a PR against
`dev`. How a reviewed capture becomes a committed reference input without
committing the image itself is the pending generator design.

## Tuning

`run-e2e-shard.sh` accepts `DIFF_TOLERANCE` (per-channel delta, default 16/255)
and `DIFF_BUDGET` (fraction of pixels allowed to differ, default 0.02), both
read by `compare-screenshots.swift`.
