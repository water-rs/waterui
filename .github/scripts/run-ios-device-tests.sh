#!/usr/bin/env bash
# Packages a WaterUI application for the iOS simulator with the `water` CLI
# and runs the backend's native test suite inside a booted simulator.
#
# Two things are verified end to end, with no C header and no
# `dynamic_lookup` anywhere:
#
#   * the application static library `water package` produces through
#     `export_app!` really defines the `waterui_apple_main` /
#     `waterui_apple_mount` entry points the thin Swift adapter binds via
#     `@_extern(c)` — asserted on the archive's symbol table so a broken
#     packaging leg cannot turn the check green — and the packaged `.app`
#     installs and launches on the simulator; and
#   * the `native` and `native_app` libtest-mimic suites (Tests/native.rs
#     and Tests/native_app.rs, behind the `native-test` feature) run inside
#     the same booted device: `cargo nextest run --target
#     aarch64-apple-ios-sim` hands every test binary to the
#     `nextest-ios-sim.sh` target runner (`.cargo/config.toml`), so
#     UIKit-touching assertions execute natively on the platform rather
#     than the host. Plain binaries are `simctl spawn`ed bare; the
#     `native_app` harnesses are wrapped in a generated `.app` and launched
#     into a real `UIApplication` with a connected scene, one case per
#     launch.
#
# Usage:
#   WATERUI_DIR=<staged waterui checkout> run-ios-device-tests.sh [example] [simulator-udid]
#
#   [example]  a project under ${WATERUI_DIR}/examples (e.g. reminders,
#              navigation), or `ios_test_host` — the backend's own fixture,
#              a workspace member at backends/apple/Tests/IOSTestHost.
#              Default: ios_test_host. Pass `none` to run only the native
#              suite.
#
# Prerequisites: the framework checkout (this repository — the backend is
# in-tree at backends/apple), plus the `water` CLI and `cargo nextest`:
# WATER_BIN names the exact CLI build under test (PATH is the fallback, and
# the resolved path is echoed so a stale install is visible in the log).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
example="${1:-ios_test_host}"
simulator_udid="${2:-${SIMULATOR_UDID:-}}"
if [[ -z "${simulator_udid}" ]]; then
  simulator_udid="$(xcrun simctl list devices available \
    | awk -F '[()]' '/iPhone/ {print $2; exit}')"
fi
if [[ -z "${simulator_udid}" ]]; then
  echo "error: no available iPhone simulator" >&2
  exit 1
fi

# Boot if needed and wait for it to be ready; a no-op when already booted.
xcrun simctl bootstatus "${simulator_udid}" -b

if [[ "${example}" != "none" ]]; then
  waterui_dir="${WATERUI_DIR:?WATERUI_DIR must name the framework checkout}"
  if [[ "${example}" == "ios_test_host" ]]; then
    example_path="${repo_root}/backends/apple/Tests/IOSTestHost"
  else
    example_path="${waterui_dir}/examples/${example}"
  fi
  [[ -f "${example_path}/Water.toml" ]] || {
    echo "error: no example at ${example_path}" >&2; exit 1; }

  # `water package` stages the example's Rust archive as `libwaterui_app.a`
  # in the directory its `Packaged at <path>` line names: the CLI
  # unconditionally copies the run's own BuiltTarget archive beside the
  # placed `.app` BEFORE emitting the report, so the archive found there is
  # this run's artifact by the producer's own contract — no timestamp or
  # side-effect check can or needs to prove that.

  # Resolve the CLI under test explicitly: WATER_BIN wins over PATH, and the
  # resolved path is logged, so an older install (e.g. `~/.cargo/bin/water`
  # shadowing a fresh build) is visible rather than silently driving the
  # packaging step.
  water_bin="${WATER_BIN:-$(command -v water || true)}"
  [[ -n "${water_bin}" && -x "${water_bin}" ]] || {
    echo "error: no water CLI on PATH; set WATER_BIN to the build under test" >&2
    exit 1
  }
  water_bin="$(cd "$(dirname "${water_bin}")" && pwd)/$(basename "${water_bin}")"
  echo "run-ios-device-tests: water CLI is ${water_bin}"
  "${water_bin}" --version || {
    echo "error: ${water_bin} could not report its version" >&2
    exit 1
  }

  package_log="$(mktemp)"
  "${water_bin}" package --platform ios-simulator --backend apple --debug --path "${example_path}" \
    2>&1 | tee "${package_log}"
  app_path="$(sed -n 's/.*Packaged at //p' "${package_log}" | tail -n 1 \
    | sed 's/\x1b\[[0-9;]*m//g' | tr -d '\r')"
  rm -f "${package_log}"
  [[ -n "${app_path}" && -d "${app_path}" ]] || {
    echo "error: water package did not report a packaged bundle for ${example}" >&2; exit 1; }
  archive="$(dirname "${app_path}")/libwaterui_app.a"
  [[ -f "${archive}" ]] || {
    echo "error: no libwaterui_app.a beside ${app_path} for ${example};" \
      "the invoked CLI did not stage this run's archive" >&2; exit 1; }

  # The thin adapter binds `waterui_apple_mount` through `@_extern(c)` and
  # the generated app entry point calls `waterui_apple_main`; both symbols
  # are the `export_app!` contract and must be defined by this archive.
  # Use the producing Rust toolchain's reader for embedded LLVM objects.
  # Reader errors fail validation. Consume the complete symbol table so
  # an early match cannot terminate upstream processes under pipefail.
  llvm_nm="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-nm"
  [[ -x "${llvm_nm}" ]] || {
    echo "error: ${llvm_nm} is missing; run rustup component add llvm-tools-preview" >&2
    exit 1
  }
  symbol_table="$("${llvm_nm}" -gU "${archive}")"
  for symbol in _waterui_apple_main _waterui_apple_mount; do
    printf '%s\n' "${symbol_table}" | awk -v s="${symbol}" \
      '$3 == s { found = 1 } END { exit !found }' || {
      echo "error: ${symbol} is not defined by ${archive};" \
        "the packaged app cannot bind the adapter" >&2
      exit 1
    }
  done

  # Install the packaged app, then hand launch + readiness to the helper:
  # it attaches the dev.waterui log stream before launching, accepts the
  # `waterui_first_paint_ms` marker only from this launch's pid, fails on a
  # stream loss or a 30 s marker deadline, and terminates the app on every
  # path. A returned pid alone never counts as having rendered.
  xcrun simctl install "${simulator_udid}" "${app_path}"
  bundle_id="$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' "${app_path}/Info.plist")"
  "${repo_root}/.github/scripts/measure-native-launch.py" ios-simulator \
    "${simulator_udid}" "${bundle_id}"
fi

# The reference host runs once on the same simulator and leaves its
# measured values as JSON; the suite's native-layout assertions compare
# against them through WATERUI_REFERENCE_METRICS, which the target runner
# forwards into every test process, spawned or launched, as SIMCTL_CHILD_*.
# Assigned before exporting: `export X="$(...)"` would mask the
# substitution's exit status, so a failing reference build must fail here.
export WATERUI_IOS_SIM_UDID="${simulator_udid}"
reference_metrics="$("${repo_root}/.github/scripts/prepare-native-reference.sh" \
  "$(mktemp -d)/native-reference")"
export WATERUI_REFERENCE_METRICS="${reference_metrics}"

# The native assertions run inside the same simulator through the target
# runner — plain test binaries are spawned bare on the device, the
# `native_app` harnesses launched there as a `UIApplication`. cocoa-ui
# joins the same invocation: its standalone iOS-sim nextest coverage
# transfers 1:1 onto the backend's graph, and waterui-apple's
# `native-test` forwards to cocoa-ui's so one flag selects both suites.
cargo nextest run -p waterui-apple -p cocoa-ui --locked --features waterui-apple/native-test \
  --manifest-path "${repo_root}/Cargo.toml" \
  --target aarch64-apple-ios-sim
