# Competitive benchmark suite

The harness assembled for
[water-rs/waterui#1262](https://github.com/water-rs/waterui/issues/1262)
(tracked as
[water-rs/waterui#1864](https://github.com/water-rs/waterui/issues/1864)):
identical workloads implemented once per framework, measured per platform.

## Layout

- `apps/<framework>/` — exactly one project per cross-platform framework:
  `waterui`, `flutter`, `react-native`, `electron`. Each implements the
  workloads once and serves every platform leg that framework runs on.
- `<platform>/` — the leg runner, its manifest, and only the native
  contestants of that platform: Compose and Views under `android/`,
  SwiftUI/UIKit plus the XCTest runner under `apple/`, GTK 4 under
  `linux/`, WinUI 3 under `windows/`.
- `lib/` — code shared by the legs (toolchain provisioning, plus the
  canonical `paragraphs.txt` workload text).

## In-tree CLI model

`cli/` is a workspace member of this repository — framework, water CLI,
backends and Hydrolysis are one commit. Every leg provisions `water` from
`lib/toolchain.py::provision_water_cli`:

- the runner requires a clean tracked checkout (`git status --porcelain
  --untracked-files=no` must be empty) so the HEAD sha is the real source
  identity;
- the CLI is compiled once per host via `cargo install --locked --path cli
  --root benchmarks/competitive/.cache/toolchain/water-cli-<head>-<target>`,
  serialized by a file lock so concurrent legs cannot race a partial
  install; a `.provenance` stamp (head, target, reported version, binary
  sha256) is checked on reuse and a tampered binary triggers a rebuild;
- when the Android `android` backend is used, the `android-backend-revision`
  the root `Cargo.toml` declares is recorded alongside HEAD.

No out-of-tree CLI repo, revision or binary hash is pinned anywhere.

## Canonical workload spec

W1 Hello (centred label + counter), W2 Feed (10,000 lazy rows:
`Row title {i}` / `Second line of subtitle for item {i}` / avatar circle /
timestamp `{(i/60)%24}:{i%60}`), W3 Motion (200 rects wandering a 720×440
field — per-rect xorshift64 streams, waypoints eased in-out over
`1200+(i%5)*200` ms), W4 Text (50 mixed Latin/CJK/emoji paragraphs,
`lib/paragraphs.txt`), W5 capacity (W3 scene at rect counts 200→25600
doubling), W6 capacity (W2 rows × 1→64 nested sibling cells). Every
contestant uses the same 6-colour palette
(`#3B82F6 #10B981 #F59E0B #EF4444 #8B5CF6 #EC4899`), 40-unit rects, and
traps on a missing or unrecognized workload selector rather than silently
running W1.

## Method

- Every reported metric is the **median of ≥5 runs**; min/max (spread),
  the successful-run count and the raw samples are kept in the results
  JSON, and a cell below the repetition floor is refused rather than
  emitted.
- Frame timing is captured contestant-agnostically (vsync/present events,
  not framework instrumentation); memory is OS-observed, not
  self-reported.
- **Numbers taken on an emulator or VM without a hardware GPU are
  development numbers only.** A frame-time or memory figure measured on a
  software rasterizer (llvmpipe, lavapipe, SwiftShader, WARP, Basic Render
  Driver) is not evidence. Each leg requires actual renderer evidence for
  the measured process before a sample counts; package-size figures are
  GPU-independent and are recorded everywhere.
- On Android the same WaterUI app is two contestants — backend `android`
  (the Kotlin runtime) and `hydrolysis` — shown as separate columns; a
  contestant that fails to build or launch produces a failed cell with
  diagnostics, never a skipped one.

## Committed vs generated

Third-party platform directories a pinned generator produces unmodified
are created at build time by the runner, not committed: `flutter create`
platforms, the React Native Android project (`@react-native-community/cli`
init at the manifest pin, plus the authored `android-override/` files).
Committed third-party content is limited to files the harness authored or
changed (workload sources, config channels, pod/workspace changes) and the
lockfiles that pin versions. Gradle wrapper scripts/jar are materialised
at build time from the manifest-pinned, SHA256-verified distribution.

Results are not committed.
