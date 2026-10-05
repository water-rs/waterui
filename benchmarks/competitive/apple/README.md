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
  (200 → 25600), 1 s settle + 4 s hold per step, self-paced inside the
  measure window on `dev.bench.begin`. iOS contestants only.
- W6 Feed capacity: the W2 fling program over rows with 1 → 64 nested
  text+shape children per row, same step cadence as W5. iOS only.

W5/W6 step boundaries are recorded by the app itself: `step k n=<param>
t=<unix>` appended to `tmp/bench-steps.log` plus a `dev.bench.step`
Darwin post — identical in every contestant. Frame timing is external:
`bench.py` attaches `xctrace record` (Animation Hitches + Time Profiler
+ Logging) to the app process during the measure window — launch is
gated on the `dev.bench.recorder` handshake so the attach binds at the
app's birth; on ios-device the host has no notify channel and arms a
`bench-recorder-go` sentinel in the runner container instead —
exports the
`hitches-frame-lifetimes` table for presented-frame intervals and
`time-sample` for CPU, then slices both by the logged step times. No
in-app CADisplayLink: it cannot see Flutter/RN render-thread stalls the
same way. Report per step: frames, p50/p99 frame interval, % inside the
8.33 ms (120 Hz) and 16.67 ms (60 Hz) budgets, CPU ms/frame; the
capacity number is the largest step sustaining ≥ 99% in budget.

Every app reads `--bench-workload W1..W6` from launch arguments and marks
first frame with `BENCH_READY`; W1's button is `increment-button` for
accessibility. Values are byte-identical across contestants via the
shared corpus.

## Metrics

Per contestant × workload: cold launch time (XCTApplicationLaunchMetric),
memory physical peak + steady (XCTMemoryMetric), CPU time (XCTCPUMetric),
hitch ratio + animation signposts (XCTHitchMetric /
scrollingAndDecelerationMetric — the signpost metric attaches only on the
scrolling workloads W2/W4/W6; on W1/W3 it emitted zero rows).
Render-server cost is measured alongside every cell, identically for every
contestant: on ios-sim/macos a `ps -o time` sampler tracks the app and the
Core Animation render server (sim `backboardd`, macOS `WindowServer`); on
ios-device a second `xctrace record --template 'Time Profiler'` attaches to
`backboardd`, and `Animation Hitches` attaches to the app for
presented-frame counts. Package size = arm64 `.app` bytes on
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
- Uniform auto-drive fallback (`--drive auto`): where the snapshot cost
  exceeds the cap for a contestant, the *same* fallback applies to every
  contestant — the runner sleeps while the app under test scrolls itself
  through the identical program (8 down-steps + 2 up-steps on the same
  content). `--drive` is recorded per row; report renders per-drive
  subsections. Programmatic scrolling emits no UIKit deceleration
  signposts for *any* contestant, so the auto cells carry memory /
  launch / CPU but no OSSignpost-Scroll data.
- `XCTHitchMetric` produces no samples for any contestant on the iOS
  Simulator (no GPU frame telemetry); hitch/fps data belongs to the
  physical-device leg on the M1 host.
- On macOS the swipe driver could not gesture-scroll the AppKit app:
  `swipeUp` on the app window fails with an element-matching error in
  every rep, while the same drive works on the other five macOS
  contestants. W2/W4 for AppKit were re-run with the same uniform
  `--drive auto` fallback; the swipe error rows are kept verbatim in the
  results JSON and the report's error table.
- `water package --platform ios --backend apple --release --unsigned`
  (cli dev @ `12a5ed6`, which carries feat/212) produces the unsigned
  production device artifact (`CODE_SIGNING_ALLOWED=NO`, static Rust in
  the main executable, UIDeviceFamily [1,2]); the M1 host re-signs
  inside-out before install.
- This VM has no signing identity, so App Store `.ipa` thinning is not
  possible here; package size is the arm64 `.app` bundle, and device
  thinning data belongs to the M1 host pass.

## Upstream bugs hit while building (pre-#281, historical)

The following were defects of the pre-migration Swift apple-backend and
the cli revision that packaged it (cli `cbe43f2`/`d17d0dd`, waterui
`1b80ef53`, apple-backend `c7908d7e`/`702019b`). All are obsolete under
the Rust + objc2 host — the current waterui-bench builds with plain
`water package`, and none of the workarounds below exist on this branch.
They are retained only as provenance for the pre-#281 measurement rows
(held outside git as session attachments).

1. **apple-backend** `Sources/WaterUI/Components/Metadata/Interaction/
   WuiContextMenu.swift` — `WuiContextMenuAccessoryWindow` (UIWindow
   subclass added upstream in 3a958f5) lacks `required init?(coder:)`,
   an iOS compile error under Xcode 26/Swift 6. Patched locally in the
   SPM checkout; `scripts/fix-waterui-backend.sh` re-applies the
   `@available(*, unavailable) required init?(coder:)` after
   regeneration so the manifest stays reproducible.
2. **waterui ↔ apple-backend ABI drift** — waterui dev `384ea059`
   (in `f8d4d954`+, still in `25569538`) adds `is_empty: bool` to
   `WuiSubView` (`ffi/src/components/layouting/layout.rs:350-362`,
   growing the stride 32→40 B). apple-backend tag 0.3.0 (`8f2fd7b`)
   predates it; a scaffold pinned at the tag crashes on first layout
   (`EXC_BAD_ACCESS` inside `waterui_layout_place_subviews`). The fix
   (`WuiSubView.is_empty` reader) exists only on apple-backend dev —
   carried by pinning `revision = 702019b6…`. cli#206 (lock-seed) now
   emits `kind = revision` at backend dev HEAD natively, so current
   scaffolds are safe; pre-fix scaffolds and locks must not be reused.
3. **water CLI** (water-rs/cli) — the "required Apple linker flags"
   update writes *per-platform* `OTHER_LDFLAGS` into the generated
   `project.pbxproj` cumulatively: after an ios-simulator package, a
   macOS package links against `UIKit`/iphonesimulator rpaths and fails
   (`ld: framework 'UIKit' not found`). Workaround encoded in
   manifest.json: restore the pristine scaffold `project.pbxproj`
   (`scripts/pristine/`) before each platform's package.
4. **water CLI** — `package --platform ios` hard-requires a signing
   team (`src/apple/platform.rs` → `toolchain.rs`), which prevents
   producing an unsigned or ad-hoc device artifact from CI on unmodified
   dev. Fixed on branch `feat/212-package-unsigned` @ `d17d0dd`
   (`--unsigned` = `DeviceSigning::Unsigned` → `CODE_SIGNING_ALLOWED=NO`
   on the device build; `--unsigned` is rejected for ios-simulator and
   combined with `--distribution`; `--release`/`--debug` live in a
   flattened `ProfileArgs` group, bare `water package` builds release).
   The device drop ships waterui-bench.app built with that cli; the M1
   host re-signs it inside-out like the other contestants.
5. **fmt** (react-native pods, fmtlib/fmt#4740) — fmt 11.0.2 (iOS) /
   12.1.0 (macOS) consteval guards break on Apple clang 21; patched in
   `Pods/fmt/include/fmt/base.h` via `post_install` hooks in both
   Podfiles.
6. **water CLI** Water.lock guard (`src/project_model/framework.rs`)
   treats crates.io drift on capability-graph transitive deps as a
   locked-dependency change; the shared `../apps/waterui` workspace
   member resolves against the checkout's own `Cargo.lock`, so no
   per-app lock or pins exist.
7. **water CLI** shared build cache: after `target/package` was removed
   between platform builds, the ios-device xcodebuild's "Build Rust
   Library" step failed with "Cargo still reports a shared dylib unit
   as fresh … `deps/waterui_dylib.d` names no source under this unit's
   manifest root". Only `water gc build-cache --shared-target` (wipes
   `~/.water/build_cache/target/shared`, ~8 GB rebuild) recovered it.
   Whether the entry-owning objc2 packaging still exposes this is open —
   the current contestant builds once per platform with no cache
   surgery.
