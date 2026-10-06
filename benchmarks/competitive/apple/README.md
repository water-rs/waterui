# Competitive benchmark — Apple targets

Implements the Apple slice of water-rs/waterui#1262: identical workloads
W1–W4 across WaterUI (apple-backend), SwiftUI, UIKit, AppKit, Flutter,
React Native (iOS + macOS) and Electron (macOS only), measured with the
same XCUITest harness and XCTest metrics for every contestant.

## Layout

```
apps/
  waterui-bench/    WaterUI app — `water create --channel dev` project on the
                    Rust + objc2 apple backend (apple-backend#281); exact
                    framework/backend identity = checkout HEAD sha (in-tree cli/)
  native-bench/     xcodegen project: SwiftUI + UIKit (iOS), SwiftUI +
                    AppKit (macOS) targets + the shared XCUITest runner
  flutter-bench/    Flutter app (iOS simulator debug — release is not
                    supported on the simulator; macOS release)
  rn-bench/         React Native app (RN 0.81.6 iOS, react-native-macos
                    0.81.9) via CocoaPods workspaces
  electron-bench/   Electron app; packaged by scripts/build-electron-mac.sh
xctest/Sources/Runner/BenchTests.swift
                    shared test runner: XCTApplicationLaunchMetric,
                    XCTMemoryMetric, XCTCPUMetric, XCTHitchMetric,
                    XCTOSSignpostMetric.scrollingAndDecelerationMetric
shared/             workload constants every app reads (row counts,
                    rect counts, paragraph corpus, palette, seeds)
bench.py            single entry point (uv run, stdlib only)
manifest.json       every framework/toolchain pin + per-contestant build
                    commands, bundle ids, artifact + harness paths
scripts/            build helpers (electron packaging, results cleanup)
```

## Commands

```
uv run bench.py build --platform ios-sim      # build + stage every contestant
uv run bench.py build --platform macos
uv run bench.py build --platform ios-device   # unsigned iphoneos artifacts

uv run bench.py run-local --platform ios-sim --repeats 5   # simulator run
uv run bench.py run-local --platform macos   --repeats 5   # this Mac
uv run bench.py report                        # build/report.md from JSON

# On the M1 device host (prebuilt artifacts only, never compiles):
uv run bench.py device --artifacts <dir-with-unsigned-apps+runner>
```

`device` mode takes `fcntl.flock` locks at `/tmp/device-locks/<udid>.lock`
(`mac.lock` for the Mac itself), interleaves contestant order across
repeats, provisions
free-team profiles for each bundle id via a generated empty app target
(`-allowProvisioningUpdates`), signs inside-out with `launchctl asuser`
codesigning, installs via `devicectl`, and measures with the same
XCUITest runner (6 App IDs total: 5 contestant apps + 1 shared
`dev.bench.runner`).

## Workloads (per issue #1262)

- W1 Hello: centred label + counter button, 8 s
- W2 Feed: lazy list of 10 000 rows (avatar circle, two text lines,
  trailing HH:MM), 12 s of driven flings (8 down + 2 up)
- W3 Motion: 200 rounded rects animating position/rotation/opacity,
  per-rect duration 1200 + (i % 5) × 200 ms ease-in-out, 12 s
- W4 Text: 50 mixed Latin/CJK/emoji paragraphs scrolling, 12 s, same
  corpus from `shared/paragraphs.txt`
- W5 Motion capacity: the W3 scene with the rect count doubled per step
  (200 → 25600) — one launch renders one step, pinned by
  `-bench-step N` (the ladder value). iOS contestants only.
- W6 Feed capacity: the W2 fling program over rows with 1 → 64 nested
  text+shape children per row, same step protocol as W5. iOS only.

Every cell is measured over METHOD's window (`../WORKLOADS.md`):
[first owned present + `harness.warmup_ms`, + the workload's
`duration_s`]. `dev.bench.ready` is a readiness signal only: the runner
starts the drive at ready + warmup, marking it with a `dev.bench`
`drive-begin` Points of Interest signpost, and holds the contestant for
`duration_s` + `harness.anchor_tolerance_ms` (`measure-end`). XCTest runs a
measure block once more than `iterationCount` and discards the first
run; that discarded run is the warmup and the recorded one is the drive
and hold, so each happens exactly once.

Frame timing is external: on macOS and ios-device `bench.py` runs
`xctrace record --all-processes` (Animation Hitches + Points of Interest;
plus Logging on ios-device) for every contestant. Each recorder is armed
— xctrace's own "recording started" line — before the runner's latched
recorder-go is released, so it covers the app from its launch. The
contestant's processes are those whose executable lives inside its
bundle (trace TOC); a `hitches-frame-lifetimes` row is the contestant's
present (at start + duration) when its swap composited a
`hitches-updates` row from one of them. The window is computed on that
trace's clock, the drive-begin signpost must lie within
`anchor_tolerance_ms` of its start, and frame statistics are clipped to it
exactly, whenever the runner's measured span closes. On ios-device
`time-sample` gives the app and `backboardd` CPU over the same window; on
macOS the host `ps` sampler's window is the trace window carried to the
host clock through the drive-begin instant both clocks record.
`bench.py attribution --trace T --app A --platform P --max-fps F` prints a
trace's attribution evidence (owned pids, the swap join, owned presents
next to every present on the same display). No in-app CADisplayLink: it
cannot see Flutter/RN render-thread stalls the same way. Report per step:
frames, p50/p99 frame interval, % inside the 8.33 ms (120 Hz) and
16.67 ms (60 Hz) budgets, CPU ms/frame; the capacity number is the
largest step sustaining ≥ 99% in budget.

Every app reads `-bench-workload w1..w6` from launch arguments and posts
`dev.bench.ready.<bundle-id>.<w>` when the workload view first appears
(SwiftUI `onAppear`, UIKit/AppKit `viewDidAppear`, WaterUI `on_appear`,
Flutter's first frame of the workload page, React Native's first content
appearance); W1's button is `increment-button` for accessibility. Values
are byte-identical across contestants via the shared corpus.

## Metrics

Per contestant × workload: cold launch time (XCTApplicationLaunchMetric),
memory physical peak + steady (XCTMemoryMetric), CPU time (XCTCPUMetric),
hitch ratio + animation signposts (XCTHitchMetric /
scrollingAndDecelerationMetric — the signpost metric attaches only on the
scrolling workloads W2/W4/W6; on W1/W3 it emitted zero rows).
Render-server cost is measured alongside every cell, identically for every
contestant: on ios-sim/macos a `ps -o time` sampler tracks the app and the
Core Animation render server (sim `backboardd`, macOS `WindowServer`); on
ios-device the all-process Animation Hitches recording's `time-sample`
rows give the app and `backboardd` CPU. Package size = arm64 `.app` bytes on
disk. Median / min / max / all samples, ≥ 5 repeats, written to
`build/results-*.json`; `report` renders tables with × WaterUI ratios.
Each contestant's first run of a session is a discarded warm-up — the
first-ever XCUITest attach can fail inside the framework
(`Failed to initialize for UI testing: XCTFuture`) and must not land on a
measured rep.

## Known measurement limitations

- iOS Simulator has no real vsync pacing — frame/hitch numbers are
  directional only; physical-device numbers come from `device` mode.
- Flutter on the iOS Simulator ships only a debug build (Flutter tooling
  rejects `--release`/`--profile` for the simulator target) — recorded
  in the results and flagged in the report, never silently substituted.
- Scroll gestures are synthesized as raw HID coordinate drags
  (`XCUICoordinate.press(forDuration:thenDragTo:)`), not element queries:
  `app.swipeUp()` resolves the app element via "descendants matching type
  Window" plus a per-gesture quiescence wait, which stalls past ~117 s on
  a scrolling 10k-row list — this was the cause of the WaterUI W2 device
  failures (UIKit's idiomatic `dequeueReusableCell` table hit the same
  stall, so it was the runner's query, not the backend's list —
  `WuiList` virtualises and exposes no accessibility-container
  overrides). Coordinate drags emit touch events without any AX query.
  `run_one` uses one 900 s cap for every contestant; cells that still
  time out are recorded with the exact xcodebuild error.
- There is no in-app scroll fallback: every scrolling cell is driven by
  OS-level input only (swipe on iOS device, host-side CGEvent wheel on
  macOS and the simulator). A contestant that cannot be driven fails the
  cell — it is never silently measured scrolling itself.
- `XCTHitchMetric` produces no samples for any contestant on the iOS
  Simulator (no GPU frame telemetry); hitch/fps data belongs to the
  physical-device leg on the M1 host.
- On macOS and the iOS Simulator the scroll drive is the host-side
  wheel driver (`wheel`): bench.py posts CGEvent scroll-wheel detents
  into the contestant's window on each `dev.bench.begin` — the XCTest
  `swipe` path only exists on ios-device, where real gestures can be
  synthesized.
- `water package --platform ios --backend apple --release --unsigned`
  (the in-tree CLI built from this checkout) produces the unsigned
  production device artifact (`CODE_SIGNING_ALLOWED=NO`, static Rust in
  the main executable, UIDeviceFamily [1,2]); the device host re-signs
  inside-out before install.
- This VM has no signing identity, so App Store `.ipa` thinning is not
  possible here; package size is the arm64 `.app` bundle, and device
  thinning data belongs to the M1 host pass.

## Upstream bugs hit while building

1. **fmt** (react-native pods, fmtlib/fmt#4740) — fmt 11.0.2 (iOS) /
   12.1.0 (macOS) consteval guards break on Apple clang 21; patched in
   `Pods/fmt/include/fmt/base.h` via `post_install` hooks in both
   Podfiles.
2. **water CLI** Water.lock guard (`src/project_model/framework.rs`)
   treats crates.io drift on capability-graph transitive deps as a
   locked-dependency change; the shared `../apps/waterui` workspace
   member resolves against the checkout's own `Cargo.lock`, so no
   per-app lock or pins exist.
3. **water CLI** shared build cache: a `target/package` clean between
   platform builds once broke the ios-device xcodebuild's "Build Rust
   Library" step with "Cargo still reports a shared dylib unit
   as fresh … `deps/waterui_dylib.d` names no source under this unit's
   manifest root" (recoverable only via `water gc build-cache
   --shared-target`, an ~8 GB rebuild). The leg builds once per platform
   and does not clean `target/package` between builds.
