# Issue #282 — backend comparison harness

Source-only harness for the paired Swift versus Rust/objc2 comparison in
water-rs/apple-backend#282. No measurement results have been accepted.

## Exact inputs

| Side | apple-backend | waterui | CLI |
| --- | --- | --- | --- |
| old | `c7908d7e3b7ec6b4d00f2af294be4ea5404ec92c` | `8cf506ce4ecce482878983e74eb2723a46f1b9bb` | `3927ddc56039512777db22f0d4fd8b2f3f71a1d0` |
| new | Required `BENCH282_NEW_APPLE_BACKEND_SHA` | Required `BENCH282_NEW_WATERUI_SHA` | Required `BENCH282_NEW_CLI_SHA` |

The old triplet is the pairing resolved by Nightly E2E run 36561768882.
The coordinator supplies all three exact new-side SHAs after the signed
fixes land. The former backend placeholder `7088fd9` is explicitly rejected,
even if supplied through the environment. Fixed old pins cannot be overridden.
The warm cloud host was prepared with old CLI `3927ddc` and new CLI `63d5ef6`.
The native-only Apple configuration requires CLI `223487133c` and a later
framework pin. Supply the full 40-character coordinator-approved SHAs;
the harness never expands these abbreviations or infers a binary's source.

Use Python 3.11+ and `uv`. The script declares its maintained TOML writer,
`tomli-w==1.2.0`, inline; reading uses standard-library `tomllib`.

## Reusable preparation and immutable run inputs

`setup` prepares checkouts, CLI tools and an owned compatible simulator.
`finalize-inputs` is the explicit boundary before scaffold: it verifies all
inputs and snapshots the host toolchain into the measurement run. It also
performs preparation, so an already provisioned host can call it directly.

Before any scaffold directory, parity result or measurement record exists,
finalization may reconcile **all three new-side pins**: backend, framework and
CLI. Old-side pins and receipts remain immutable. Each exact checkout path must
be owned by bench282, a standalone repository with one worktree, the expected
origin and a completely clean index/worktree including untracked files.
The harness fetches the supplied SHA, performs an ordinary detached checkout,
then rechecks HEAD and cleanliness. It never resets, forces, retries candidates
or replaces another worktree. The stable backend symlink continues to refer
to this same owned path and is the only untracked framework entry permitted,
after its exact target is verified. Ordinary `setup` cannot change prepared pins.

Before mutation, a typed `PreparedInputs` plan checks every existing source
checkout's ownership, origin, HEAD and complete tracked/untracked cleanliness,
both backend links, and both installed CLI receipts. A dirty checkout, wrong
receipt, old-side change or started run fails before fetch, checkout, receipt
replacement or input-state writes. Only after the whole preflight succeeds are
the planned exact commits fetched and checked out.

Finalization invalidates unmeasured source/lock/parity/package state while
archiving the previous pins, receipts and derived state in `input_history`.
Each history entry records the requested pins and whether preparation reached
finalization; interrupted preparation retains its preflight entry. Superseded
CLI receipts remain in `tool_history`. Scaffold marks the run started
before making files. Preparation cannot change a started run; scaffold,
parity and measurement reject a supplied pin that differs from finalized inputs.

An existing CLI is reused only when its exact source SHA and binary SHA-256
match a stored receipt. `water --version` does not expose a source commit.
For tools already installed by the coordinator, pass `--cli-provenance` to
import the coordinator's build provenance JSON: top-level `old` and `new`
objects, each with `source_sha` (40 hex) and `binary_sha256` (64 hex).
Those values must come from the actual successful source build and its output;
the current checkout or a version string alone does not certify a binary.
The receipt is stored under `state.tools` and the binary is checked again at
the finalized-input gate. Missing or mismatched provenance fails without
reinstalling an existing binary. Before starting the run, an explicit
`finalize-inputs --cli-provenance ...` may replace a stale **new-side** receipt
with a coordinator build receipt matching both the requested new CLI SHA and
the actual installed binary hash. The coordinator places that built binary at
the existing new toolchain path before finalization. A changed source checkout
or `--version` output never relabels an old receipt, and a supplied mismatching
receipt is rejected even when a stored receipt exists. No VM transfer or binary
replacement is performed by this source-only handoff.

When no binary exists, exact unreleased source is installed with
`cargo install --locked --path <checkout> --root <toolchain>` in that checkout.
Its ordinary Cargo target and locks are preserved. The already installed
cloud CLIs take the receipt-reuse path, so no target clone or duplicate CLI
compile is needed. No foreign cache transfer or alternate target directory
is introduced. Preparation command durations, outcomes and source SHAs go to
`logs/preparation.jsonl`, separate from `results/`; new install receipts also
retain the install command and duration. Toolchains are outside every measured
cold-clean root; measured app dependency units are still deleted in full.

## Simulator compatibility

Selection uses `simctl list runtimes --json`: an available iOS runtime must
provide `supportedDeviceTypes`, with device identifiers and `productFamily`.
Only a device explicitly advertised as an iPhone by that runtime is eligible.
The highest numeric runtime version is selected, with a deterministic device
identifier ordering within its supported set. Missing evidence fails with the
runtime identifier; it never infers compatibility from a global device list.

The harness creates its own simulator and records its UDID, runtime identifier
and device type before awaiting `simctl bootstatus <UDID> -b`. A recorded
owned device is reused only if the runtime still supports its type and the
native device list contains that exact available UDID/type/runtime tuple.
Precreated devices and names are not ownership evidence. The old bare
`simulator_udid` field alone is not adopted. There are no candidate retries or
readiness sleeps. The actual cloud JSON schema remains a pilot validation:
the source tests use explicit runtime-response fixtures, not a local simulator.

## Subjects and manifest compatibility

Both sides use the generated fresh app and the same form source from
`waterui@8cf506ce:examples/form`. Fresh source must match byte for byte.
Form source hashes must match the staged pinned source on both sides.
Parity fingerprints all app source files and four Cargo lockfiles.
Each measurement checks the current source, exact backend binding, checkout
HEADs and tracked checkout changes; each record carries its input digest.

The old CLI requires `[package] type = "app"` and `[backends.apple] scheme`;
the harness also persists its exact `[backends.apple] backend_path`. The new
CLI rejects the retired app-mode keys through
`project_model/app_mode.rs::APP_MODE_KEYS`, and generated projects carry no
`[backends.*]` table at all: its local backend is the `backends/apple`
checkout under `waterui_path` — here the harness-owned symlink whose exact
target is verified. New manifests therefore record `waterui_path` and
`[package]` only; any `backends` table or `package.type` fails validation,
as does a wrong or non-owned backend link.
All generated/edited TOML uses parsed tables and the serializer. The exact
backend binding is read back and validated on both sides, including paths
containing quotes.
These schemas were audited in local CLI source at old pin 3927ddc, new-side
feat/223-embedded-mode 1e65f438 and current origin/dev; the eventual new
pin still requires the cloud pilot.

Incremental builds change one rendered header string:
`WaterUI Demo` → `WaterUI Demo!`, or
`WaterUI Form Examples` → `WaterUI Form Examples!`.
This changes reachable view data, without an unused constant or syntactically
broken source replacement. Every cold build, preview pair and release package
starts with the original label. Both platforms apply the same one-line edit.

Old `water build` builds the Rust library; its native link occurs during
package/run. New `water build` includes the managed native host.
Reports retain these distinct build scopes.

## Dedicated user and cold state

The driver must execute as the real macOS account `bench282`, whose owned
home is exactly `/Users/bench282`. `BENCH282_ROOT` must resolve inside it.
A HOME override under the primary UID is rejected. An unclaimed, nonempty
`~bench282/.water` is rejected. Cache paths and redirected Cargo/rustup
homes are checked against the dedicated account boundary before cleanup.

Cold cleanup runs the pinned CLI's scoped clean and rejects a nonzero exit.
It then removes this account's complete `~/.water/build_cache`, Xcode
DerivedData and SwiftPM compiled cache, plus project target, apple, ffi,
DerivedData and .water directories. Registry downloads and installed
toolchains remain. Cleanup records removed paths and allocated bytes.

Build disk metrics sum non-overlapping roots: the entire build cache,
DerivedData, SwiftPM cache, and project target/apple/ffi/DerivedData.
This includes shared dependency units and generated native outputs.
Disk is allocated bytes; app and executable package sizes are logical bytes.

## Event and ownership protocol

1. Install the release simulator bundle when applicable.
2. Start live `log stream --style ndjson --level info`, filtered broadly
   by subsystem `dev.waterui`.
3. Await the stream's attach header through its pipe before spawning the app.
4. Spawn the app and obtain its PID. Structured events already received or
   buffered in the pipe remain available.
5. Accept `waterui_first_paint_ms=` only from an event with that exact
   `processID` and subsystem. Foreign PIDs cannot satisfy the marker.
6. Collect all 20 RSS samples at 0.5-second intervals; steady RSS is the
   median of the final five. An early process exit invalidates the leg.
7. Reap the owned app and log stream on success, exceptions and interruption.
   Simulator termination and uninstall are registered before launch; cleanup
   continues even if another cleanup callback fails.

There is no log-show replay or readiness sleep. The only sleep is the
specified RSS sampling interval. First paint is the backend's process-start
to first-paint marker; marker-observed wall time is a separate diagnostic.
Fresh process launch is measured; filesystem cache eviction is not claimed.
Simulator launches terminate any previous instance of this benchmark bundle.
Each subprocess has a bound of at most 1800 seconds; launch shares a single
deadline across attachment, spawn, marker and RSS, plus bounded cleanup.

## Execution plan and complete reports

`uv run --script drive.py plan` emits a fail-fast shell plan:

- Per sample, subject and build platform: old cold → old incremental →
  new cold → new incremental.
- Per preview platform: old cold → old warm → new cold → new warm.
- Per subject and package platform: old package → old launch where supported →
  new package → new launch where supported.

A package's launch is immediate: no intervening cold wipe can remove its
artifact. Incremental, warm-preview and launch legs require the immediately
preceding successful matching leg, including sample ID. A historical result
cannot certify current warm state. Duplicate sample identities are refused.
Artifacts are located from the CLI's complete `Packaged at` line (including
spaces); executable names come from Info.plist.

The manifest defines the complete required metric matrix, including build
disk use. There are 200 measurement commands and 54 paired scalar cells.
Every cell requires exactly samples 1–5 on both sides, no duplicates, every
required metric, finite nonnegative values, successful runs, matching input
digests and toolchain evidence. Missing whole metrics, legs, sides and empty
results fail. Reports include median/min/max and new/old ratios only for
complete cells; a zero old median cannot produce a ratio. Commands, declared
app/dependency features and build scopes accompany the JSON report.

## Cloud pilot handoff

The coordinator owns provisioning and execution on the same macOS VM.
Provision the dedicated account and its own rustup/Xcode/uv environment,
then place this directory under its home. Run from a shell owned by
`bench282`, with all three final pins supplied by the coordinator:

```sh
export BENCH282_ROOT=/Users/bench282/bench282
# Export BENCH282_NEW_APPLE_BACKEND_SHA, BENCH282_NEW_WATERUI_SHA,
# and BENCH282_NEW_CLI_SHA as exact commits.
# cli-provenance.json contains the existing coordinator build receipts.
uv run --script drive.py finalize-inputs --cli-provenance cli-provenance.json
uv run --script drive.py scaffold old
uv run --script drive.py scaffold new
uv run --script drive.py parity
uv run --script drive.py measure old fresh cold-build --platform macos --sample 1
uv run --script drive.py measure new fresh cold-build --platform macos --sample 1
```

The two pilot records belong to a pilot run root. Use a separate run root
for the full 200-command pass; repeating sample 1 in the pilot root fails.
The paired pilot validates CLI/framework compatibility, generated app and
lockfile behavior, actual NDJSON/header delivery and simulator access under
the dedicated user before accepting numbers. Device packaging is unsigned
.app output. A headless account without working CoreSimulator must be
provisioned with a supported dedicated login/bootstrap by the coordinator.

## Source-only checks

```sh
uv run --no-project --with tomli-w==1.2.0 python -B -m unittest discover -s benches/issue-282 -v
```

Tests use temporary text fixtures and mocked subprocess/measurement calls.
They exercise stream fragmentation and early markers, PID filtering,
attachment-before-spawn on both platforms, cleanup failures, manifest
serialization, real label edits, full matrix failures, exact plan coverage,
warm-state predecessors and cache ownership. They do not build, launch
applications, drive a GUI or generate performance numbers.
