#!/usr/bin/env bash
# iOS Simulator target runner for the Rust backend's tests.
#
# Wired up through `target.aarch64-apple-ios-sim.runner` in
# `.cargo/config.toml`, which `cargo test` and `cargo nextest run` honour:
# each built test binary is handed here and executed *inside* a booted
# iPhone simulator via `simctl spawn`, so `UIKit`-touching tests run
# natively on the platform — compiling for the simulator is never the test.
#
# `spawn` runs the Mach-O directly: the suite creates UIKit objects
# (`UIView`, `UILabel`, …) without reaching `UIApplication`, so no `.app`
# bundle is needed. The spawned process's exit status is simctl's, so a
# failing test fails the step.
#
# Usage:  nextest-ios-sim.sh <test-binary> [test args…]
# Env:    WATERUI_IOS_SIM_UDID  REQUIRED — the UDID of the owned iPhone
#         simulator the runner boots. CI creates the device itself and
#         exports the UDID; the runner never picks a device implicitly,
#         so a run can never land on a stale or foreign simulator.
set -euo pipefail

binary="${1:?usage: nextest-ios-sim.sh <test-binary> [test args…]}"
shift

if [[ -z "${WATERUI_IOS_SIM_UDID:-}" ]]; then
    echo "nextest-ios-sim: WATERUI_IOS_SIM_UDID is unset — this runner" >&2
    echo "requires the UDID of a simulator the caller owns and creates" >&2
    echo "(CI provisions one per run). Create one, e.g.:" >&2
    echo "  xcrun simctl create dev.waterui-tests <SimDeviceType> <SimRuntime>" >&2
    exit 1
fi
device="$WATERUI_IOS_SIM_UDID"

# Boot if needed and wait for it to be ready; a no-op when already booted.
# nextest parses this runner's stdout for its list/run protocol, so
# bootstatus chatter must stay off it.
xcrun simctl bootstatus "$device" -b >&2

if [[ -n "${WATERUI_REFERENCE_METRICS:-}" ]]; then
    export SIMCTL_CHILD_WATERUI_REFERENCE_METRICS="$WATERUI_REFERENCE_METRICS"
fi
exec xcrun simctl spawn "$device" "$binary" "$@"
