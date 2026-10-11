# Apple planes device harness

This extends the Apple external-frame harness with the recorded-content
scenarios shared with Android. `static:512:1:0:plane` retains one opaque
512×512 picture while an unrelated indicator changes each frame.
`static:512:1:0:engine` adds an empty translucent layer above it: pixels
stay identical, but `TranslucentAbove` prevents plane promotion.
The static scenarios create no external-frame producer and play no media.

`--measure --run-id <identity>` settles for five seconds and measures twenty
seconds of display-link callbacks. The app writes and syncs
`Documents/planes-result.json`, then exits. The host waits for that process
exit through `devicectl --console` before copying the report.

The existing bench `energy::Meter` reads the iOS kernel's cumulative
`proc_pid_rusage` v6 `ri_energy_nj` counter. The report includes its delta
in joules, joules per rendered frame and average watts. **This is process
energy.** It excludes the display and compositor server, and cannot establish
whole-device savings from moving work to the compositor. An unavailable or
unchanged counter fails the run. No replacement meter is selected.

Frame times cover the host's scene update and synchronous `Engine::render`
call. Callback cadence is reported separately; neither is GPU execution
time. Memory includes engine GPU/CPU bytes, current process physical
footprint, and lifetime peak footprint. Serious or critical thermal state
invalidates the run. The host sets screen brightness to its minimum.
Before creating the engine, the host waits for a thermal-state notification
until the phone is nominal: starting at fair can exhaust the thermal
headroom during the window. A fifteen-minute cooling deadline
fails the run; it never starts a measurement without thermal recovery.

## Build on the Mac mini

Check the 60 GB available-space floor before each build. From the repository
root, with no other Cargo command running:

```sh
CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_INCREMENTAL=0 cargo build --locked \
  --release --target aarch64-apple-ios \
  --manifest-path gpu/examples/apple_planes/Cargo.toml
cd gpu/examples/apple_planes/ios
xcodegen generate
security unlock-keychain -p ""
xcodebuild -project CherenkovPlanes.xcodeproj -scheme CherenkovPlanes \
  -sdk iphoneos -configuration Release -destination 'generic/platform=iOS' \
  -derivedDataPath build DEVELOPMENT_TEAM=4AZ53N9R83 \
  PRODUCT_BUNDLE_IDENTIFIER=dev.cherenkov.bench CODE_SIGN_STYLE=Automatic \
  -allowProvisioningUpdates build
```

The host uses the UIScene lifecycle required by iOS 27.

## Run on the Mac mini

`run.py` measures ABAB three times followed by BABA, with A promoted and B
in-engine. Each window takes the shared iPhone's advisory lock, installs
the signed app, launches it, and copies its UUID-matched report. It releases
the lock between windows. A lock wait of fifteen minutes stops the job.

From the repository root, choose a fresh output directory outside git:

```sh
mkdir /tmp/cherenkov-apple-measurement
mkfifo /tmp/cherenkov-apple-measurement/done
nohup python3 gpu/examples/apple_planes/run.py \
  --app "$PWD/gpu/examples/apple_planes/ios/build/Build/Products/Release-iphoneos/CherenkovPlanes.app" \
  --out /tmp/cherenkov-apple-measurement/results \
  --completion-fifo /tmp/cherenkov-apple-measurement/done \
  </dev/null >/tmp/cherenkov-apple-measurement/run.log 2>&1 &
```

Wait with `cat /tmp/cherenkov-apple-measurement/done`. The FIFO receives one
completion record and closes; `results/completion.json` preserves the same
record. Keep result JSON, logs and measurement reports outside git.
After diagnosing an interrupted run, `--start-index N` resumes at its first
unfinished window without changing the order. Every preceding result must
exist, and completed results are never overwritten. Preserve the failed
attempt's log and completion record before resuming in the same directory.
