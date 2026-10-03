#!/usr/bin/env bash
# Nightly e2e shard. For every example assigned to this shard: package it in
# release mode, record the .app and executable sizes, launch the packaged app
# directly, capture the self-reported first-paint marker, sample peak RSS, and
# verify a settled screenshot — a capture must carry real content (not a
# uniform fill), and when a recorded baseline exists under backends/apple/Tests/E2EBaselines
# it must match within the compare budget. A `.skip` marker next to a baseline
# opts an example out of pixel comparison only; launch and content are still
# verified. With RECORD=1, captures are written to ${RECORD_DIR} for the
# baseline-publish job instead of being compared. Failures are collected so one
# broken example does not hide the state of the rest; the script exits nonzero
# if any example failed.
#
# EXAMPLES (comma-separated) names an explicit example list instead of a shard
# slice — the targeted e2e workflow uses it to re-run a fixed set; the nightly
# leaves it unset and keeps the checksum sharding below.
#
# PREBUILT_DIR names a directory of already-packaged bundles — the nightly's
# per-platform build job packages every example once and hands the shards the
# result as one artifact (#218). When it is set the shard skips `water
# package` and the silence-wait entirely: app_path resolves to the bundle under
# ${PREBUILT_DIR}/apps/<example>/ and the build job's package log is copied in
# as the run log so the uploaded artifacts look as they did when the shard
# packaged inline. Package-size recording moves to the build job with the
# package; when PREBUILT_DIR is unset the script still packages inline and
# records sizes itself (the targeted e2e workflow runs that way).
#
# The shard measures the release package, never a debug `water run`: the
# packaged artifact is what users ship, so size, startup, memory, and pixel
# parity all describe the production binary.
set -euo pipefail

platform="${PLATFORM:-${1:-}}"
shard_index="${SHARD_INDEX:-${2:-}}"
shard_total="${SHARD_TOTAL:-${3:-}}"
workspace="${GITHUB_WORKSPACE:-$(pwd)}"
waterui_dir="${WATERUI_DIR:-${workspace}}"
logs_dir="${EXAMPLE_LOG_DIR:-${workspace}/e2e-logs}"
shots_dir="${SHOTS_DIR:-${workspace}/e2e-shots}"
baselines_dir="${BASELINES_DIR:-${workspace}/backends/apple/Tests/E2EBaselines}"
record_dir="${RECORD_DIR:-${workspace}/e2e-baselines}"
record="${RECORD:-0}"
prebuilt_dir="${PREBUILT_DIR:-}"
parity_budgets="${PARITY_BUDGETS:-${baselines_dir}/parity-budgets.json}"
reference_dir="${workspace}/backends/apple/Tests/E2EReference"

if [[ "${platform}" != "ios" && "${platform}" != "macos" ]]; then
  echo "::error::Unsupported platform '${platform}'. Expected ios or macos."
  exit 1
fi
if [[ -z "${shard_index}" || -z "${shard_total}" ]]; then
  echo "::error::SHARD_INDEX and SHARD_TOTAL are required"
  exit 1
fi
if (( shard_index < 0 || shard_total <= 0 || shard_index >= shard_total )); then
  echo "::error::Invalid shard configuration: index=${shard_index}, total=${shard_total}"
  exit 1
fi
if [[ ! -d "${waterui_dir}" ]]; then
  echo "::error::Missing waterui checkout at ${waterui_dir}"
  exit 1
fi
if [[ "${platform}" == "ios" && -z "${SIMULATOR_UDID:-}" ]]; then
  echo "::error::SIMULATOR_UDID is required for iOS runs"
  exit 1
fi
if [[ -n "${prebuilt_dir}" && ! -d "${prebuilt_dir}/apps" ]]; then
  echo "::error::PREBUILT_DIR '${prebuilt_dir}' has no apps/ directory."
  exit 1
fi

mkdir -p "${logs_dir}" "${shots_dir}"
startup_entries="${logs_dir}/.startup-${platform}-${shard_index}.entries"
memory_entries="${logs_dir}/.memory-${platform}-${shard_index}.entries"
size_entries="${logs_dir}/.size-${platform}-${shard_index}.entries"
: > "${startup_entries}"
: > "${memory_entries}"
: > "${size_entries}"
if [[ "${record}" == "1" ]]; then
  mkdir -p "${record_dir}/${platform}"
fi

all_examples=()
while IFS= read -r example; do
  all_examples+=("${example}")
done < <("${workspace}/.github/scripts/discover-examples.sh" "${waterui_dir}")
declare -a shard_examples=()

# The `${arr[@]+...}` form: under the system bash 3.2 an empty array expands to
# an unbound-variable error, and shards legitimately assign no examples.
if [[ -n "${EXAMPLES:-}" ]]; then
  # An explicit list selects exactly those examples; a name that discovery did
  # not find is a typo, not an empty shard, so it fails the run fast.
  IFS=',' read -ra requested_examples <<< "${EXAMPLES}"
  for example in ${requested_examples[@]+"${requested_examples[@]}"}; do
    example="$(printf '%s' "${example}" | tr -d '[:space:]')"
    [[ -n "${example}" ]] || continue
    known=0
    for candidate in ${all_examples[@]+"${all_examples[@]}"}; do
      if [[ "${candidate}" == "${example}" ]]; then
        known=1
        break
      fi
    done
    if (( known == 0 )); then
      echo "::error::Requested example '${example}' is not a runnable example under ${waterui_dir}/examples."
      exit 1
    fi
    shard_examples+=("${example}")
  done
else
  for example in ${all_examples[@]+"${all_examples[@]}"}; do
    checksum=$(printf '%s' "${example}" | cksum | awk '{print $1}')
    if (( checksum % shard_total == shard_index )); then
      shard_examples+=("${example}")
    fi
  done
fi

if (( ${#shard_examples[@]} == 0 )); then
  echo "Shard ${shard_index}/${shard_total} has no examples for ${platform}."
  echo "{}" > "${logs_dir}/startup-times-${platform}-${shard_index}.json"
  echo "{}" > "${logs_dir}/memory-${platform}-${shard_index}.json"
  if [[ -z "${prebuilt_dir}" ]]; then
    echo "{}" > "${logs_dir}/package-sizes-${platform}-${shard_index}.json"
  fi
  exit 0
fi

echo "Running ${#shard_examples[@]} examples on ${platform}: ${shard_examples[*]}"

if [[ "${platform}" == "ios" ]]; then
  # A `simctl launch` opens the app over whatever is currently frontmost, and
  # iOS answers with the status-bar "back to <app>" breadcrumb — pixels the
  # parity compare counts. The harness terminates every app it starts before
  # the next launch, so the only breadcrumb source is a foreign app left
  # frontmost outside the run (a diagnostic launch, an aborted previous run).
  # Rebooting is the deterministic way to guarantee SpringBoard is frontmost
  # for the first launch; every launch afterwards inherits that clean state.
  xcrun simctl shutdown "${SIMULATOR_UDID}" >/dev/null 2>&1 || true
  if ! xcrun simctl boot "${SIMULATOR_UDID}" >/dev/null 2>&1; then
    echo "::error::Failed to boot simulator ${SIMULATOR_UDID} for a clean launch state."
    exit 1
  fi
  xcrun simctl bootstatus "${SIMULATOR_UDID}" -b

  # Pin the status bar for the whole shard — WaterUI and twin captures alike —
  # so the clock, battery and signal pixels compare equal on every shot. The
  # override persists until it is cleared at the end of the shard (the EXIT
  # trap covers failure paths too).
  if ! xcrun simctl status_bar "${SIMULATOR_UDID}" override \
    --time "9:41" \
    --batteryState charged --batteryLevel 100 \
    --wifiMode active --wifiBars 3 \
    --cellularMode active --cellularBars 4 \
    --dataNetwork wifi; then
    echo "::error::Failed to set the status bar override on ${SIMULATOR_UDID}."
    exit 1
  fi
  trap 'xcrun simctl status_bar "${SIMULATOR_UDID}" clear >/dev/null 2>&1 || true' EXIT
fi

# The capture helpers below re-invoke the checker sources constantly: the
# settle loop runs a pixel compare once per poll (up to the 90 s deadline,
# per example and per reference capture), content/compare run again per
# example and twin, and every macOS frame first asks window-id for the
# window number. `swift <file>` re-parses and type-checks the source on
# every call — measured here at ~1.5 s for `content` on a real capture
# vs ~0.02 s for the compiled binary — and the pixel loop itself runs
# interpreted. The sources stay unchanged; they are compiled once per job
# into scratch space and the binaries are reused for every call. Nothing
# persists between jobs — the directory lives under TMPDIR and the EXIT
# trap removes it.
swift_tools_dir="$(mktemp -d "${TMPDIR:-/tmp}/e2e-swift-tools.XXXXXX")"
if [[ "${platform}" == "ios" ]]; then
  trap 'rm -rf "${swift_tools_dir}"; xcrun simctl status_bar "${SIMULATOR_UDID}" clear >/dev/null 2>&1 || true' EXIT
else
  trap 'rm -rf "${swift_tools_dir}"' EXIT
fi
if ! swiftc -O -o "${swift_tools_dir}/compare-screenshots" \
     "${workspace}/.github/scripts/compare-screenshots.swift"; then
  echo "::error::Failed to compile compare-screenshots.swift."
  exit 1
fi
if [[ "${platform}" == "macos" ]]; then
  if ! swiftc -O -o "${swift_tools_dir}/window-id" \
       "${workspace}/.github/scripts/window-id.swift"; then
    echo "::error::Failed to compile window-id.swift."
    exit 1
  fi
fi

# One frame from the current platform target. macOS captures need the pid that
# owns the window; the running example is the default, the SwiftUI reference
# host passes its own.
capture_frame() {
  local target="$1"
  local pid="${2:-${app_pid}}"
  if [[ "${platform}" == "ios" ]]; then
    xcrun simctl io "${SIMULATOR_UDID}" screenshot "${target}" >/dev/null
  else
    local window_id
    window_id="$("${swift_tools_dir}/window-id" "${pid}")"
    screencapture -x -o -l"${window_id}" "${target}"
  fi
}

# Captures until every frame across SETTLE_WINDOW_S seconds agrees with the
# window's first frame within DIFF_BUDGET=0.01 — the app has settled once its
# output stops changing — or the deadline passes, in which case the last
# frame is kept. Fails when no frame could be captured.
#
# Two consecutive frames are not enough: SwiftUI materialises a glassEffect
# in a second commit the render server lands after first paint, and the
# intermediate state is pixel-static, so a short agreement check photographs
# it mid-materialisation. Nightly run 35435178963 caught the liquid_glass
# twin that way — two reference captures 3.6 s apart agreed while the media
# card was still translucent, and the 0.1688 parity diff was entirely that
# card (#256). Measured on an iPhone 18 Pro / iOS 27.0 simulator (frame every
# ~0.5 s from launch): the glass reaches its final state ~1 s after the
# first-paint marker is observed and the screen never changes again through
# 30 s. 10 s is ~10x that measurement and ~2.7x the static plateau the
# failing run demonstrated, while leaving the 90 s deadline untouched.
SETTLE_WINDOW_S=10
capture_settled() {
  local target="$1"
  local pid="${2:-}"
  local anchor="${shots_dir}/.settle-anchor.png"
  local window_start=0
  local deadline=$((SECONDS + 90))
  rm -f "${anchor}"
  while (( SECONDS < deadline )); do
    if capture_frame "${target}" ${pid:+"${pid}"} && [[ -f "${target}" ]]; then
      if [[ -f "${anchor}" ]] && \
         DIFF_BUDGET=0.01 "${swift_tools_dir}/compare-screenshots" \
           compare "${anchor}" "${target}" "${shots_dir}/.settle-diff.png" >/dev/null 2>&1; then
        if (( SECONDS - window_start >= SETTLE_WINDOW_S )); then
          rm -f "${anchor}"
          return 0
        fi
      else
        # First usable frame, or the screen changed: the window restarts here.
        cp "${target}" "${anchor}"
        window_start=${SECONDS}
      fi
    fi
    sleep 1
  done
  rm -f "${anchor}"
  if [[ ! -f "${target}" ]]; then
    return 1
  fi
  echo "::warning::${example} never settled to a stable frame; using the last capture."
}

# ── SwiftUI parity ──────────────────────────────────────────────────────────
# Examples with a registered twin in backends/apple/Tests/E2EReference are also compared
# against the twin rendered live by the reference host on the same runner —
# the backend must stay pixel-faithful to what SwiftUI produces for the same
# layout, not only to its own recorded baseline. The twin registry is read
# from Twins.swift so a new twin joins the sweep automatically.
twin_names=()
if [[ -f "${reference_dir}/Sources/Twins.swift" ]]; then
  while IFS= read -r twin; do
    twin_names+=("${twin}")
  done < <(sed -n 's/.*case "\([^"]*\)":.*/\1/p' "${reference_dir}/Sources/Twins.swift" | sort -u)
fi

has_twin() {
  local t
  for t in ${twin_names[@]+"${twin_names[@]}"}; do
    [[ "${t}" == "$1" ]] && return 0
  done
  return 1
}

# Built lazily on the first twinned example in the shard; a shard with no
# twins never pays for it.
reference_app=""
build_reference_host() {
  [[ -n "${reference_app}" ]] && return 0
  local out_dir="${shots_dir}/.reference-host"
  if "${reference_dir}/build-reference-host.sh" "${platform}" "${out_dir}" \
      >"${logs_dir}/${platform}-reference-build.log" 2>&1; then
    reference_app="${out_dir}/E2EReference.app"
  else
    echo "::error::Failed to build the SwiftUI reference host."
    tail -n 60 "${logs_dir}/${platform}-reference-build.log" || true
    return 1
  fi
}

# Waits for the reference host to report that it has drawn its first frame,
# reading the marker it logs on the `dev.waterui` subsystem.
#
# The capture used to follow a fixed `sleep 2`. A SwiftUI cold launch in the
# simulator regularly needs longer than that, and `capture_settled` accepts two
# consecutive frames that agree — which a not-yet-drawn blank screen does with
# itself on the first iteration. The run then compared a blank twin against a
# correct WaterUI render and reported it as a parity regression (multi_window
# on iOS, 0.0490 against a 0.0200 budget, run 35406819126), the same shape as
# the launch-failure bug #147 fixed above, one step later in the sequence.
# Waiting on the host's own signal removes the race rather than widening the
# constant.
wait_for_reference_first_paint() {
  local marker_log="$1"
  for _ in $(seq 1 60); do
    if grep -q "waterui_reference_first_paint_ms=" "${marker_log}" 2>/dev/null; then
      return 0
    fi
    sleep 1
  done
  # A very fast first paint can still beat the stream attach; the marker is in
  # the persisted log store, so replay the recent window before giving up.
  if [[ "${platform}" == "ios" ]]; then
    xcrun simctl spawn "${SIMULATOR_UDID}" log show --last 2m \
      --predicate 'subsystem == "dev.waterui"' --style compact \
      >> "${marker_log}" 2>/dev/null || true
  else
    log show --last 2m --predicate 'subsystem == "dev.waterui"' \
      --style compact >> "${marker_log}" 2>/dev/null || true
  fi
  grep -q "waterui_reference_first_paint_ms=" "${marker_log}" 2>/dev/null
}

# Launches the reference host for `example`, waits for its first paint,
# captures to $1, and shuts it down again. The reference never runs
# concurrently with the example under test, so the runner's single screen needs
# no window choreography.
capture_reference() {
  local target="$1" example="$2" title="$3"
  build_reference_host || return 1
  local ref_marker_log="${logs_dir}/${platform}-${example}-ref-marker.log"
  : > "${ref_marker_log}"
  if [[ "${platform}" == "ios" ]]; then
    # A failed install/launch used to slide through and screenshot the home
    # screen — which then compared as a bogus ~95% parity regression (#147).
    xcrun simctl install "${SIMULATOR_UDID}" "${reference_app}" >/dev/null || return 1
    xcrun simctl spawn "${SIMULATOR_UDID}" log stream --level info \
      --predicate 'subsystem == "dev.waterui"' --style compact \
      > "${ref_marker_log}" 2>/dev/null &
    local ref_stream_pid=$!
    sleep 1
    xcrun simctl launch "${SIMULATOR_UDID}" dev.waterui.E2EReference \
      -E2EExample "${example}" -E2ETitle "${title}" >/dev/null || {
      kill "${ref_stream_pid}" 2>/dev/null || true
      return 1
    }
    local rc=0
    if wait_for_reference_first_paint "${ref_marker_log}"; then
      capture_settled "${target}"
      rc=$?
    else
      echo "::error::${example}: the SwiftUI twin never reported a first paint on ios; refusing to compare against an unrendered reference."
      rc=1
    fi
    kill "${ref_stream_pid}" 2>/dev/null || true
    xcrun simctl terminate "${SIMULATOR_UDID}" dev.waterui.E2EReference >/dev/null 2>&1 || true
    return ${rc}
  else
    log stream --predicate 'subsystem == "dev.waterui"' --style compact \
      > "${ref_marker_log}" 2>/dev/null &
    local ref_stream_pid=$!
    sleep 1
    "${reference_app}/Contents/MacOS/E2EReference" \
      -E2EExample "${example}" -E2ETitle "${title}" \
      >>"${logs_dir}/${platform}-${example}-ref.log" 2>&1 &
    local ref_pid=$!
    local rc=0
    if wait_for_reference_first_paint "${ref_marker_log}"; then
      capture_settled "${target}" "${ref_pid}"
      rc=$?
    else
      echo "::error::${example}: the SwiftUI twin never reported a first paint on macos; refusing to compare against an unrendered reference."
      rc=1
    fi
    kill "${ref_stream_pid}" 2>/dev/null || true
    kill "${ref_pid}" 2>/dev/null || true
    wait "${ref_pid}" 2>/dev/null || true
    return ${rc}
  fi
}

# The allowed diff fraction for this twin on this platform. A "<platform>@<os
# major>" section wins over the plain platform key, which wins over the strict
# default: "ios@27" records the navigation exception — SwiftUI's private bar
# collapses the large title for a stacked search bar on iOS 27 while stock
# UINavigationBar keeps it for identical public inputs, so the twin's extra
# diff there is a platform behavior change, not a backend regression.
parity_os_major=""
parity_budget() {
  local budget=""
  if [[ -f "${parity_budgets}" ]]; then
    if [[ -z "${parity_os_major}" ]]; then
      if [[ "${platform}" == "ios" ]]; then
        parity_os_major="$(xcrun simctl list devices --json | python3 -c '
import json, re, sys
for runtime, group in json.load(sys.stdin)["devices"].items():
    if any(d.get("udid") == sys.argv[1] for d in group):
        m = re.search(r"iOS-(\d+)", runtime)
        if m:
            print(m.group(1))
        break
' "${SIMULATOR_UDID}")" || return 1
        if [[ -z "${parity_os_major}" ]]; then
          echo "::error::parity_budget: cannot derive iOS major: SIMULATOR_UDID=${SIMULATOR_UDID} not found in 'xcrun simctl list devices', or its runtime name has no iOS-<major> match." >&2
          return 1
        fi
      else
        parity_os_major="$(sw_vers -productVersion | cut -d. -f1)" || return 1
        if [[ -z "${parity_os_major}" ]]; then
          echo "::error::parity_budget: cannot derive macOS major: 'sw_vers -productVersion' returned no version." >&2
          return 1
        fi
      fi
    fi
    budget="$(python3 -c '
import json, sys
budgets = json.load(open(sys.argv[1]))
platform, major, example = sys.argv[2], sys.argv[3], sys.argv[4]
value = ""
if major:
  value = budgets.get(f"{platform}@{major}", {}).get(example, "")
if value == "":
  value = budgets.get(platform, {}).get(example, "")
print(value)
' "${parity_budgets}" "${platform}" "${parity_os_major}" "$1")" || return 1
  fi
  echo "${budget:-${DIFF_BUDGET:-0.02}}"
}

# `water package` emits `Packaged at <path>` on success; resolve the bundle it
# names. There is deliberately no search fallback: the build cache under
# `~/.water/build_cache` is shared by every example and survives across runs,
# so "the first .app found" can be another example's bundle or a stale one.
find_packaged_app() {
  local log_file="$1" app_path
  app_path="$(sed -n 's/.*Packaged at //p' "${log_file}" | tail -1 | sed 's/\x1b\[[0-9;]*m//g' | tr -d '\r')"
  [[ -n "${app_path}" && -d "${app_path}" ]] && echo "${app_path}"
}

declare -a failures=()
declare -a report=()

for example in ${shard_examples[@]+"${shard_examples[@]}"}; do
  mem_cell="—"
  fp_cell="—"
  size_cell="—"
  example_path="${waterui_dir}/examples/${example}"
  [[ -d "${example_path}" ]] || example_path="${waterui_dir}/backends/apple/Examples/${example}"
  run_log="${logs_dir}/${platform}-${example}.log"
  marker_log="${logs_dir}/${platform}-${example}-marker.log"
  shot="${shots_dir}/${platform}-${example}.png"
  rm -f "${shot}"

  echo "::group::${platform} example ${example}"

  # The product name is Water.toml's package.name; the running process is the
  # executable inside the built .app bundle.
  product="$(sed -n 's/^name = "\([^"]*\)"$/\1/p' "${example_path}/Water.toml" | head -1)"
  bundle_id="$(sed -n 's/^bundle_identifier = "\([^"]*\)"$/\1/p' "${example_path}/Water.toml" | head -1)"
  package_platform="macos"
  [[ "${platform}" == "ios" ]] && package_platform="ios-simulator"

  : > "${marker_log}"
  app_path=""
  runner_pid=""
  stuck=0
  if [[ -n "${prebuilt_dir}" ]]; then
    # The build job packaged this example already; its package log stands in
    # as the run log so the artifacts look as they did when the shard built
    # inline, and a missing bundle keeps the same unsupported/skip semantics
    # an inline failure produced.
    if [[ -f "${prebuilt_dir}/logs/${platform}-${example}.log" ]]; then
      cp "${prebuilt_dir}/logs/${platform}-${example}.log" "${run_log}"
    else
      : > "${run_log}"
    fi
    app_path="$(find "${prebuilt_dir}/apps/${example}" -mindepth 1 -maxdepth 1 \
      -name '*.app' -type d -print -quit 2>/dev/null || true)"
  else
    : > "${run_log}"
    water package --platform "${package_platform}" --backend apple --release \
      --path "${example_path}" > "${run_log}" 2>&1 &
    runner_pid=$!

    # The wait covers `water package`'s cold release build, not just a launch:
    # without a shared sccache a first-in-shard example compiles for tens of
    # minutes, and release codegen runs longer than debug. The bound is
    # therefore on *silence*, not duration — a build that is writing to the log
    # or running compiler processes is making progress and must not be killed
    # (#140). A dead runner still short-circuits, and a hard ceiling caps hung
    # cases.
    built=0
    log_size=-1
    silent_since=${SECONDS}
    deadline=$((SECONDS + 2700))
    while (( SECONDS < deadline )); do
      if ! kill -0 "${runner_pid}" 2>/dev/null; then
        if wait "${runner_pid}"; then built=1; fi
        break
      fi
      new_size=$(stat -f%z "${run_log}" 2>/dev/null || echo -1)
      if (( new_size != log_size )); then
        log_size=${new_size}
        silent_since=${SECONDS}
      elif pgrep -f 'xcodebuild|swiftc|swift-frontend|cargo|rustc|clang' >/dev/null 2>&1; then
        silent_since=${SECONDS}
      elif (( SECONDS - silent_since > 300 )); then
        stuck=1
        break
      fi
      sleep 2
    done

    if (( built == 1 )); then
      app_path="$(find_packaged_app "${run_log}")"
    fi
  fi

  if [[ -n "${app_path}" ]]; then
    app_bytes="$(find "${app_path}" -type f -exec stat -f%z {} + | awk '{s+=$1} END {print s}')"
    if [[ "${platform}" == "macos" ]]; then
      executable="${app_path}/Contents/MacOS/$(basename "${app_path}" .app)"
    else
      executable="${app_path}/$(basename "${app_path}" .app)"
    fi
    executable_bytes="$(stat -f%z "${executable}")"
    if [[ -z "${prebuilt_dir}" ]]; then
      printf '  "%s": { "app_bytes": %s, "executable_bytes": %s },\n' \
        "${example}" "${app_bytes}" "${executable_bytes}" >> "${size_entries}"
    fi
    size_cell="$(awk -v b="${app_bytes}" 'BEGIN{printf "%.1f MB", b/1048576}')"
    echo "::notice::${example} packaged: .app=${app_bytes}B executable=${executable_bytes}B (${platform})"
  fi

  if [[ -z "${app_path}" ]]; then
    if [[ -n "${runner_pid}" ]] && kill -0 "${runner_pid}" 2>/dev/null; then
      if (( stuck == 1 )); then
        echo "::error::No build progress for 5 minutes packaging ${example} (${platform}); declaring the runner stuck."
        failures+=("${example}: package stuck")
        report+=("| \`${example}\` | package stuck | — | — | — | ${mem_cell} |")
      else
        echo "::error::Packaging ceiling (45 min) exceeded for ${example} (${platform})."
        failures+=("${example}: package timeout")
        report+=("| \`${example}\` | package timeout | — | — | — | ${mem_cell} |")
      fi
      kill "${runner_pid}" 2>/dev/null || true
    else
      if grep -qi "unsupported for" "${run_log}"; then
        # The CLI refused a platform-impossible configuration (e.g. a CEF
        # WebView on iOS) — by-design, not a defect (#154). Skipped, not failed.
        echo "::notice::${example} is unsupported on ${platform}; skipping."
        report+=("| \`${example}\` | skipped (unsupported) | — | — | — | ${mem_cell} |")
      else
        echo "::error::water package failed for ${example} (${platform})."
        failures+=("${example}: package")
        report+=("| \`${example}\` | package failed | — | — | — | ${mem_cell} |")
      fi
    fi
    printf '  "%s": null,\n' "${example}" >> "${startup_entries}"
    printf '  "%s": null,\n' "${example}" >> "${memory_entries}"
    if [[ -z "${prebuilt_dir}" ]]; then
      printf '  "%s": null,\n' "${example}" >> "${size_entries}"
    fi
    tail -n 120 "${run_log}" || true
    if [[ -n "${runner_pid}" ]]; then
      wait "${runner_pid}" || true
    fi
    echo "::endgroup::"
    continue
  fi

  # Launch the packaged release app directly. The log stream attaches BEFORE
  # the launch — inside the simulator on iOS, where the app's logd actually
  # lives — so the first-paint marker cannot be lost to an attach race. A
  # missing marker after the grace window is data (the app ran, the marker
  # pipeline broke), not a launch failure.
  stream_pid=""
  app_pid=""
  exec_name="$(basename "${app_path}" .app)"
  if [[ "${platform}" == "macos" ]]; then
    log stream --predicate 'subsystem == "dev.waterui"' --style compact \
      > "${marker_log}" 2>/dev/null &
    stream_pid=$!
    sleep 1
    "${app_path}/Contents/MacOS/${exec_name}" >/dev/null 2>&1 &
    app_pid=$!
    sleep 1
    if ! kill -0 "${app_pid}" 2>/dev/null; then
      kill "${stream_pid}" 2>/dev/null || true
      echo "::error::${example} packaged app exited during launch (${platform})."
      failures+=("${example}: launch")
      report+=("| \`${example}\` | launch failed | — | — | ${size_cell} | ${mem_cell} |")
      printf '  "%s": null,\n' "${example}" >> "${startup_entries}"
      printf '  "%s": null,\n' "${example}" >> "${memory_entries}"
      echo "::endgroup::"
      continue
    fi
  else
    if ! xcrun simctl install "${SIMULATOR_UDID}" "${app_path}"; then
      echo "::error::${example} packaged app failed to install in the simulator."
      failures+=("${example}: install")
      report+=("| \`${example}\` | install failed | — | — | ${size_cell} | ${mem_cell} |")
      printf '  "%s": null,\n' "${example}" >> "${startup_entries}"
      printf '  "%s": null,\n' "${example}" >> "${memory_entries}"
      echo "::endgroup::"
      continue
    fi
    xcrun simctl spawn "${SIMULATOR_UDID}" log stream --level info \
      --predicate 'subsystem == "dev.waterui"' --style compact \
      > "${marker_log}" 2>/dev/null &
    stream_pid=$!
    sleep 1
    app_pid="$(xcrun simctl launch "${SIMULATOR_UDID}" "${bundle_id}" | awk -F': ' '{print $2}')"
    if [[ -z "${app_pid}" ]]; then
      kill "${stream_pid}" 2>/dev/null || true
      echo "::error::${example} packaged app failed to launch in the simulator."
      failures+=("${example}: launch")
      report+=("| \`${example}\` | launch failed | — | — | ${size_cell} | ${mem_cell} |")
      printf '  "%s": null,\n' "${example}" >> "${startup_entries}"
      printf '  "%s": null,\n' "${example}" >> "${memory_entries}"
      echo "::endgroup::"
      continue
    fi
  fi

  startup_ms=""
  no_first_paint=0
  for _ in $(seq 1 30); do
    startup_ms="$(sed -n 's/.*waterui_first_paint_ms=\([0-9][0-9]*\).*/\1/p' "${marker_log}" | head -1)"
    [[ -n "${startup_ms}" ]] && break
    sleep 1
  done
  if [[ -z "${startup_ms}" ]]; then
    # A very fast first paint can still beat the stream attach; the marker is
    # in the persisted log store, so replay the recent window from the same
    # domain the stream reads before declaring the marker missing.
    if [[ "${platform}" == "ios" ]]; then
      xcrun simctl spawn "${SIMULATOR_UDID}" log show --last 2m \
        --predicate 'subsystem == "dev.waterui"' --style compact \
        >> "${marker_log}" 2>/dev/null || true
    else
      log show --last 2m --predicate 'subsystem == "dev.waterui"' \
        --style compact >> "${marker_log}" 2>/dev/null || true
    fi
    startup_ms="$(sed -n 's/.*waterui_first_paint_ms=\([0-9][0-9]*\).*/\1/p' "${marker_log}" | head -1)"
  fi
  if [[ -n "${startup_ms}" ]]; then
    echo "::notice::${example} first paint in ${startup_ms} ms (${platform}, release)"
    printf '  "%s": %s,\n' "${example}" "${startup_ms}" >> "${startup_entries}"
    fp_cell="${startup_ms} ms"
  else
    echo "::error::${example} did not report a first-paint time"
    printf '  "%s": null,\n' "${example}" >> "${startup_entries}"
    no_first_paint=1
  fi

  if ! capture_settled "${shot}"; then
    echo "::error::Could not capture a screenshot for ${example}."
    kill "${stream_pid}" 2>/dev/null || true
    if [[ "${platform}" == "ios" ]]; then
      xcrun simctl terminate "${SIMULATOR_UDID}" "${bundle_id}" >/dev/null 2>&1 || true
      xcrun simctl uninstall "${SIMULATOR_UDID}" "${bundle_id}" >/dev/null 2>&1 || true
    else
      kill "${app_pid}" 2>/dev/null || true
    fi
    failures+=("${example}: capture")
    report+=("| \`${example}\` | capture failed | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    printf '  "%s": null,\n' "${example}" >> "${memory_entries}"
    echo "::endgroup::"
    continue
  fi

  # A live app that never reports `waterui_first_paint_ms` is a launch
  # failure, not a launch: the capture above may show the home screen, so
  # record the example as failed instead of "launched".
  if (( no_first_paint )); then
    kill "${stream_pid}" 2>/dev/null || true
    if [[ "${platform}" == "ios" ]]; then
      xcrun simctl terminate "${SIMULATOR_UDID}" "${bundle_id}" >/dev/null 2>&1 || true
      xcrun simctl uninstall "${SIMULATOR_UDID}" "${bundle_id}" >/dev/null 2>&1 || true
    else
      kill "${app_pid}" 2>/dev/null || true
    fi
    failures+=("${example}: no first paint")
    report+=("| \`${example}\` | no first paint | — | — | ${size_cell} | ${mem_cell} |")
    printf '  "%s": null,\n' "${example}" >> "${memory_entries}"
    echo "::endgroup::"
    continue
  fi

  # Post-launch memory footprint: peak resident set sampled over a short idle
  # window while the app is still up. Simulator apps are host processes and
  # `simctl launch` returned the host pid, so `ps` covers both platforms
  # directly.
  peak_rss=0
  for _ in $(seq 1 4); do
    rss="$(ps -o rss= -p "${app_pid}" 2>/dev/null | tr -d ' ' || true)"
    if [[ -n "${rss}" ]] && (( rss > peak_rss )); then peak_rss="${rss}"; fi
    sleep 0.5
  done
  if (( peak_rss > 0 )); then
    printf '  "%s": %s,\n' "${example}" "$(( peak_rss * 1024 ))" >> "${memory_entries}"
    mem_cell="$(awk -v b="${peak_rss}" 'BEGIN{printf "%.0f MB", b/1024}')"
  else
    printf '  "%s": null,\n' "${example}" >> "${memory_entries}"
    mem_cell="—"
  fi

  kill "${stream_pid}" 2>/dev/null || true
  if [[ "${platform}" == "ios" ]]; then
    xcrun simctl terminate "${SIMULATOR_UDID}" "${bundle_id}" >/dev/null 2>&1 || true
    xcrun simctl uninstall "${SIMULATOR_UDID}" "${bundle_id}" >/dev/null 2>&1 || true
  else
    kill "${app_pid}" 2>/dev/null || true
  fi

  if ! "${swift_tools_dir}/compare-screenshots" content "${shot}"; then
    echo "::error::Captured screenshot for ${example} is blank."
    failures+=("${example}: blank")
    report+=("| \`${example}\` | blank capture | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    echo "::endgroup::"
    continue
  fi

  baseline="${baselines_dir}/${platform}/${example}.png"
  if [[ "${record}" == "1" ]]; then
    cp "${shot}" "${record_dir}/${platform}/${example}.png"
    report+=("| \`${example}\` | recorded | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
  elif [[ -f "${baselines_dir}/${platform}/${example}.skip" ]]; then
    report+=("| \`${example}\` | launched (compare skipped) | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
  elif [[ -f "${baselines_dir}/${platform}/${example}.visual" ]]; then
    # `.visual` — the semantic class for output that is not pixel-stable by
    # construction (GPU-rendered). Never pixel-compared; the capture pair
    # (WaterUI + SwiftUI twin below) ships as artifacts for human review.
    report+=("| \`${example}\` | launched — VISUAL REVIEW REQUIRED | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
  elif [[ ! -f "${baseline}" ]]; then
    report+=("| \`${example}\` | launched (no baseline yet) | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
  else
    diff_image="${shots_dir}/${platform}-${example}-diff.png"
    if compare_out="$("${swift_tools_dir}/compare-screenshots" \
        compare "${baseline}" "${shot}" "${diff_image}")"; then
      report+=("| \`${example}\` | baseline match | ${compare_out#compare: } | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    else
      echo "::error::Screenshot regression for ${example}: ${compare_out}"
      failures+=("${example}: regression")
      report+=("| \`${example}\` | regression | ${compare_out#compare: } | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    fi
  fi

  # SwiftUI parity: render the twin in the reference host and compare it
  # against the example capture taken above. A `.parity-skip` marker next to
  # the baselines opts a twin out while a known divergence is worked down.
  if has_twin "${example}" && \
     [[ ! -f "${baselines_dir}/${platform}/${example}.parity-skip" ]]; then
    ref_shot="${shots_dir}/${platform}-${example}-ref.png"
    parity_diff="${shots_dir}/${platform}-${example}-parity-diff.png"
    budget="$(parity_budget "${example}")"
    # Record runs measure rather than gate: an unbounded budget keeps the
    # compare green so the true fraction lands in the report and the artifact.
    [[ "${record}" == "1" ]] && budget="1.0"
    if ! capture_reference "${ref_shot}" "${example}" "${product}"; then
      echo "::error::Could not capture the SwiftUI reference for ${example}."
      failures+=("${example}: reference capture")
      report+=("| \`${example}\` (parity) | reference failed | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    elif ! "${swift_tools_dir}/compare-screenshots" \
        content "${ref_shot}" >/dev/null 2>&1; then
      echo "::error::SwiftUI reference for ${example} captured blank."
      failures+=("${example}: reference blank")
      report+=("| \`${example}\` (parity) | reference blank | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    elif [[ -f "${baselines_dir}/${platform}/${example}.visual" ]]; then
      # `.visual`: both halves of the pair are captured and shipped as
      # artifacts; a human reviews them — GPU-rendered output is never
      # pixel-compared.
      report+=("| \`${example}\` (parity) | VISUAL REVIEW REQUIRED | pair in e2e-shots artifacts | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    elif parity_out="$(DIFF_BUDGET="${budget}" \
        "${swift_tools_dir}/compare-screenshots" \
        compare "${ref_shot}" "${shot}" "${parity_diff}" 2>&1)"; then
      parity_fraction="${parity_out#compare: }"
      parity_fraction="${parity_fraction%% *}"
      if [[ "${record}" == "1" ]]; then
        mkdir -p "${record_dir}/parity"
        printf '%s\n' "${parity_fraction}" \
          > "${record_dir}/parity/${platform}-${example}.txt"
        report+=("| \`${example}\` (parity) | recorded ${parity_fraction} | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
      else
        report+=("| \`${example}\` (parity) | within budget ${budget} | ${parity_out#compare: } | ${fp_cell} | ${size_cell} | ${mem_cell} |")
      fi
    else
      echo "::error::SwiftUI parity regression for ${example}: ${parity_out} (budget ${budget})"
      failures+=("${example}: parity regression")
      report+=("| \`${example}\` (parity) | drift over budget ${budget} | ${parity_out#compare: } | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    fi
  fi

  echo "::endgroup::"
done

# Fold the per-example measurements into a JSON object the workflow uploads.
{
  echo "{"
  sed '$ s/,$//' "${startup_entries}"
  echo "}"
} > "${logs_dir}/startup-times-${platform}-${shard_index}.json"
rm -f "${startup_entries}"

{
  echo "{"
  sed '$ s/,$//' "${memory_entries}"
  echo "}"
} > "${logs_dir}/memory-${platform}-${shard_index}.json"
rm -f "${memory_entries}"

# Package sizes are recorded by the build job in prebuilt mode — they are a
# property of the package, and the per-platform file ships inside the
# packaged-<platform> artifact.
if [[ -z "${prebuilt_dir}" ]]; then
  {
    echo "{"
    sed '$ s/,$//' "${size_entries}"
    echo "}"
  } > "${logs_dir}/package-sizes-${platform}-${shard_index}.json"
fi
rm -f "${size_entries}"

{
  echo "## E2E ${platform} — shard ${shard_index}/${shard_total}"
  echo ""
  echo "| Example | Result | Diff | First paint | .app | Peak RSS |"
  echo "| --- | --- | --- | --- | --- | --- |"
  for row in ${report[@]+"${report[@]}"}; do
    echo "${row}"
  done
} >> "${GITHUB_STEP_SUMMARY:-/dev/stdout}"

if (( ${#failures[@]} > 0 )); then
  echo "::error::${#failures[@]} example(s) failed on ${platform}: ${failures[*]}"
  exit 1
fi
