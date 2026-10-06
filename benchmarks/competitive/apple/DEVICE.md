# Device leg — physical iOS device

Run on the Apple Silicon benchmark host (Xcode at /Applications/Xcode.app).
All `.app` bundles are universal (`UIDeviceFamily [1,2]`), unsigned, and are
re-signed inside-out by the runner with the host's `Apple Development`
identity before `devicectl` install/launch.

One-line run:

```
uv run bench.py device --artifacts artifacts
```

Identity resolution (nothing author-specific is baked in):

- `--udid` defaults to the single device `xcrun devicectl list devices`
  reports as available; pass it explicitly when several are attached.
- The signing identity is the keychain's `Apple Development` cert —
  `--identity <substring>` selects among several; `--team` overrides the
  team derived from the identity.
- Locks: `/tmp/device-locks/<udid>.lock` and `mac.lock` are flocked,
  never broken. On error or signal the runner kills only its own spawned
  processes, uninstalls only the apps this run installed, and removes its
  own `device-testroot`/`provwork` dirs.
- Thermal gate: devicectl cannot report `thermalState`, so the gate
  lives in the on-device runner — `BenchTests.setUp` waits bounded on
  `ProcessInfo.thermalStateDidChangeNotification` for a nominal state
  and fails the row if the cool-down exceeds the budget; the observed
  thermal + screen max fps are recorded per row from the runner log.

| artifact | contestant | build |
|---|---|---|
| WaterUI Bench.app | WaterUI apple backend (Rust + objc2) | `water package --platform ios --backend apple --release --unsigned` (the in-tree `water` CLI from this checkout; static Rust in the main executable, unsigned). Framework/backend identity is the checkout HEAD sha — cli/, the apple backend and hydrolysis are workspace members. Plain package — the pbxproj restore, backend fix script and Water.lock dependency pins from the pre-#281 Swift-backend build are gone. |
| BenchSwiftUI.app | SwiftUI | xcodebuild `-configuration Release` iphoneos |
| BenchUIKit.app | UIKit | same project |
| Runner.app | Flutter | `flutter build ios --release` |
| RnBench.app | React Native | xcodebuild Release iphoneos, `TARGETED_DEVICE_FAMILY="1,2"` |
| BenchRunner-Runner.app | shared XCUITest runner | build-for-testing |
| BenchIOS_iphoneos26.5-arm64.xctestrun | shared test plan | build-for-testing |

The runner signs nested content inside-out, embeds the provisioning profile it
materializes via `-allowProvisioningUpdates`, and records device
model/os in the results JSON; thermal and screen max fps come from the
on-device runner's `device-record` log lines, devicectl cannot report
them.

## Capacity workloads (W5/W6)

`--workloads w5,w6` runs the capacity ladders (manifest `platforms`:
ios-sim + ios-device, all five contestants). One launch renders one step —
`bench.py` iterates the manifest's `steps` ladder, passing `-bench-step N`
per launch, and each launch's `xctrace record --template 'Animation
Hitches' --all-processes` recording is armed before the runner's
recorder-go is released, so it covers the launch and the whole measure
window; the contestant's frames are selected at export by the swap join
described in README.md (frame lifetimes whose swap carries an update from
a process inside the contestant's bundle).

Per-row results carry `capacity.steps[]` (frames, p50/p99 frame interval,
% inside the 8.33 ms / 16.67 ms budgets, CPU ms and CPU ms/frame) plus
`capacity_120hz` / `capacity_60hz` — the largest step sustaining ≥ 99% of
presented frames inside budget. A recorder that fails to arm fails the
cell with xctrace's own output; one that fails to save its trace lands
in the row's `trace_errors`.
