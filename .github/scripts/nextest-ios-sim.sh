#!/usr/bin/env bash
# iOS Simulator target runner for the Rust backend's tests.
#
# Wired up through `target.aarch64-apple-ios-sim.runner` in
# `.cargo/config.toml`, which `cargo test` and `cargo nextest run` honour:
# each built test binary is handed here and executed *inside* the owned
# iPhone simulator, so `UIKit`-touching tests run natively on the
# platform — compiling for the simulator is never the test.
#
# Two kinds of binary arrive here:
#
# * Plain binaries — stock-harness unit tests and the `native` suites — run
#   as a bare process through `simctl spawn`; the spawned process's exit
#   status is simctl's.
# * The `native_app` harnesses (`cocoa_ui::uikit::native_test`) run their
#   cases inside a real `UIApplication` with a connected window scene:
#   `UIKit` drives part of its machinery — scroll animations among it —
#   from the application's update cycle, which a bare process never
#   reaches. Such a binary embeds its `Info.plist` in the Mach-O
#   `__TEXT,__info_plist` section, and that section is what marks it. The
#   runner wraps the binary in a generated `.app`, installs it when the
#   installed copy differs, and launches it with `simctl launch --console`,
#   one case per launch. simctl forwards the arguments and the
#   application's stdout/stderr but exits 0 whatever the application's
#   status, so the harness writes its exit status to the file named by
#   `WATERUI_NATIVE_TEST_STATUS` and the runner exits with it; a launch
#   that leaves no status — the process died before the harness
#   concluded — fails. A simulator keeps one installed copy of a bundle
#   and shows one application in the foreground: nextest serializes these
#   binaries through the `ios-sim-app` test group (`.config/nextest.toml`),
#   and a lock file in the device's own data directory guards `cargo test`
#   and separate runs sharing one device. Listing (`--list`) never needs
#   the application: the harness answers it before `UIApplicationMain`,
#   through `spawn`.
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

listing=false
for argument in "$@"; do
    [[ "$argument" == "--list" ]] && listing=true
done
# Captured whole: `grep -q` exiting early would SIGPIPE `otool` and, under
# `pipefail`, misread a marked binary as unmarked.
load_commands="$(otool -l "$binary")"
if [[ "$listing" == true || "$load_commands" != *"sectname __info_plist"* ]]; then
    exec xcrun simctl spawn "$device" "$binary" "$@"
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
plist="$work/Info.plist"
# segedit's success line would reach nextest's stdout.
segedit "$binary" -extract __TEXT __info_plist "$plist" >&2
bundle_id="$(plutil -extract CFBundleIdentifier raw "$plist")"
executable="$(plutil -extract CFBundleExecutable raw "$plist")"
app="$work/$executable.app"
mkdir "$app"
cp -c "$binary" "$app/$executable"
mv "$plist" "$app/Info.plist"
status="$work/status"
export SIMCTL_CHILD_WATERUI_NATIVE_TEST_STATUS="$status"
# The simulator's HOME is the device's own CoreSimulator data directory.
lock="$(xcrun simctl getenv "$device" HOME)/.waterui-nextest-ios-sim.lock"

# Installs `app` unless the device already holds the same executable (the
# manifest is embedded in it), then launches it and blocks until it exits.
# simctl reports `<bundle-id>: <pid>` on stdout once the app exits, joined
# onto the app's last line when that line is unterminated (an aborted
# case); it goes to stderr so a case's captured stdout is only the test's.
# shellcheck disable=SC2329 # invoked by the `bash -c` that `lockf` runs below.
launch_app() {
    local device="$1" bundle_id="$2" executable="$3" app="$4"
    shift 4
    local installed
    if ! installed="$(xcrun simctl get_app_container "$device" "$bundle_id" app 2>/dev/null)" \
        || ! cmp -s "$installed/$executable" "$app/$executable"; then
        xcrun simctl install "$device" "$app" >&2
    fi
    xcrun simctl launch --console --terminate-running-process "$device" "$bundle_id" "$@" |
        while IFS= read -r line || [[ -n "$line" ]]; do
            if [[ "$line" =~ ^(.*)("$bundle_id: "[0-9]+)$ ]]; then
                printf '%s' "${BASH_REMATCH[1]}"
                printf '%s\n' "${BASH_REMATCH[2]}" >&2
            else
                printf '%s\n' "$line"
            fi
        done
}
export -f launch_app
# A run waits at most 600 s for another run on this device to release
# it; lockf exits 75 (EX_TEMPFAIL) when that bound passes.
lock_status=0
lockf -k -t 600 "$lock" "$BASH" -c 'set -euo pipefail; launch_app "$@"' launch_app \
    "$device" "$bundle_id" "$executable" "$app" "$@" || lock_status=$?
if [[ "$lock_status" -eq 75 ]]; then
    echo "nextest-ios-sim: device lock held by another run: $lock" >&2
    exit 1
elif [[ "$lock_status" -ne 0 ]]; then
    exit "$lock_status"
fi

if [[ ! -s "$status" ]]; then
    echo "nextest-ios-sim: $bundle_id exited without reporting a status —" >&2
    echo "the harness application died before its trials concluded" >&2
    exit 1
fi
exit "$(<"$status")"
