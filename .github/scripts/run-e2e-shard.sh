#!/usr/bin/env bash
# Nightly e2e shard. For every example assigned to this shard: package it in
# release mode, record the .app and executable sizes, launch the packaged app
# directly, capture the self-reported first-paint marker, sample peak RSS, and
# capture a settled screenshot. Full-frame FLIP supplies perceptual diagnostics
# and tolerances, never a claim that the UI is correct: captures and heatmaps
# require visual review. A `.skip` marker next to a baseline
# declares a first screen that cannot be made deterministic — it opts the
# example out of comparison and out of the settle window itself: one
# supervised capture after the first-paint marker, with launch and decoding
# still verified. With RECORD=1, captures are written to ${RECORD_DIR} for the
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
# PREBUILT_REFERENCE_APP supplies an already-built E2EReference.app instead
# of compiling the SwiftUI twin on the capture host. Its platform executable
# must exist; invalid explicit input fails rather than building another host.
#
# The shard measures the release package, never a debug `water run`: the
# packaged artifact is what users ship, so size, startup, memory, and visual
# comparison all describe the production binary.
#
# Requires compare-screenshots on PATH, built once by the workflows' prepare job.
#
# Requires bash >= 4.2, asserted below (#2683): the shard keeps background
# children alive by design — the marker/reference log streams and `water
# package` — and a builtin write that races a child's exit fails with EINTR
# under /bin/bash 3.2, whose SIGCHLD handler lacks SA_RESTART.
set -euo pipefail

if (( BASH_VERSINFO[0] < 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] < 2) )); then
  echo "::error::run-e2e-shard.sh needs bash >= 4.2 (found ${BASH_VERSION}); /bin/bash 3.2 lets SIGCHLD interrupt writes with EINTR (#2683)" >&2
  exit 1
fi

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
reference_dir="${workspace}/backends/apple/Tests/E2EReference"
command_deadline="${workspace}/.github/scripts/run-with-deadline.py"
receipts_file="${logs_dir}/command-receipts-${platform}-${shard_index}.jsonl"

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
: > "${receipts_file}"
if [[ "${record}" == "1" ]]; then
  mkdir -p "${record_dir}/${platform}"
fi

# A command substitution, not process substitution: it keeps discovery's exit
# status (a captured `::error` line would otherwise become an example name),
# and the shell waits the child out instead of printing while it runs.
if ! discovered="$("${workspace}/.github/scripts/discover-examples.sh" "${waterui_dir}")"; then
  printf '%s\n' "${discovered}"
  exit 1
fi

all_examples=()
while IFS= read -r example; do
  [[ -n "${example}" ]] || continue
  all_examples+=("${example}")
done <<< "${discovered}"
declare -a shard_examples=()

# The `${arr[@]+...}` form: under bash < 4.4 an empty array expands to an
# unbound-variable error, and shards legitimately assign no examples.
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

if ! compare_tool="$(command -v compare-screenshots)" || [[ ! -x "${compare_tool}" ]]; then
  echo "::error::Missing prepared compare-screenshots executable; download its prepare-job artifact and add it to PATH."
  exit 1
fi

# Temporary diagnostics for #3608 (status bar sometimes shows only the
# battery): every simctl probe is bounded and never fails the shard.
status_diag_bounded() {
  local label="$1"
  shift
  local at
  at="$("${command_deadline}" deadline --seconds 30)" || return 0
  "${command_deadline}" run --label "diag3608:${label}" \
    --deadline-at "${at}" --receipt "${receipts_file}" \
    --max-seconds 20 -- "$@" 2>&1 || echo "(${label} exited $?)"
}
status_diag_list() {
  status_diag_bounded status-bar-list xcrun simctl status_bar "${SIMULATOR_UDID}" list
}

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

  # An iOS capture is the whole display (macOS captures the app's own window),
  # so a SpringBoard notification banner lands in the frame and reads as a
  # divergence from the app's own baseline and from the SwiftUI twin. Booting
  # out the delivery daemon stops every banner for this boot, so it runs after
  # the reboot above.
  xcrun simctl spawn "${SIMULATOR_UDID}" \
    launchctl bootout system/com.apple.usernotificationsd 2>/dev/null || true

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
  {
    echo "override applied at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "--- status_bar list right after override ---"
    status_diag_list
  } > "${logs_dir}/ios-${shard_index}-status-bar-override.txt" 2>&1
  trap 'xcrun simctl status_bar "${SIMULATOR_UDID}" clear >/dev/null 2>&1 || true' EXIT
fi

# Compile the macOS launch/window helpers once per job, not per capture poll.
swift_tools_dir="$(mktemp -d "${TMPDIR:-/tmp}/e2e-swift-tools.XXXXXX")"
if [[ "${platform}" == "ios" ]]; then
  trap 'rm -rf "${swift_tools_dir}"; xcrun simctl status_bar "${SIMULATOR_UDID}" clear >/dev/null 2>&1 || true' EXIT
else
  trap 'rm -rf "${swift_tools_dir}"' EXIT
fi
if [[ "${platform}" == "macos" ]]; then
  if ! swiftc -O -o "${swift_tools_dir}/window-id" \
       "${workspace}/.github/scripts/window-id.swift"; then
    echo "::error::Failed to compile window-id.swift."
    exit 1
  fi
  if ! swiftc -O -o "${swift_tools_dir}/launch-app" \
       "${workspace}/.github/scripts/launch-app.swift"; then
    echo "::error::Failed to compile launch-app.swift."
    exit 1
  fi
  if ! swiftc -O -o "${swift_tools_dir}/display-scale" \
       "${workspace}/.github/scripts/display-scale.swift"; then
    echo "::error::Failed to compile display-scale.swift."
    exit 1
  fi
fi

# One frame from the current platform target. macOS captures need the pid that
# owns the window; the running example is the default, the SwiftUI reference
# host passes its own. Every native command runs supervised under the caller's
# shared deadline ($2): a hung simctl/screenshot/screencapture used to block
# the settle loop unbounded between deadline checks — measured at 451 s for a
# single screenshot in the Actions run behind #1539 — and a failed command
# used to be silently retried or replaced by an earlier frame. Each command
# also carries COMMAND_BOUND_S of its own: a healthy capture answers in
# seconds, so a command still running at the bound is hung — killed there,
# while shared budget remains, which is what separates a hung command from
# ordinary settle-budget exhaustion in capture_settled below.
COMMAND_BOUND_S=30
capture_frame() {
  local target="$1"
  local deadline_at="$2"
  local pid="${3:-${app_pid}}"
  if [[ "${platform}" == "ios" ]]; then
    # Removing the prior file first means a command that exits 0 without
    # writing cannot let a stale frame pass as this capture.
    rm -f "${target}"
    local diag3608="${logs_dir}/ios-${example}-status-bar-diag.txt"
    {
      echo "=== screenshot ${target##*/} start $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
      echo "--- status_bar list before ---"
      status_diag_list
    } >> "${diag3608}" 2>&1
    local shot_rc=0
    "${command_deadline}" run --label "capture:simctl-io-screenshot" \
      --deadline-at "${deadline_at}" --receipt "${receipts_file}" \
      --max-seconds "${COMMAND_BOUND_S}" \
      -- xcrun simctl io "${SIMULATOR_UDID}" screenshot "${target}" >/dev/null || shot_rc=$?
    {
      echo "=== screenshot end $(date -u +%Y-%m-%dT%H:%M:%SZ) rc=${shot_rc} ==="
      echo "--- status_bar list after ---"
      status_diag_list
      echo "--- SpringBoard status log, last 2m ---"
      status_diag_bounded springboard-log xcrun simctl spawn "${SIMULATOR_UDID}" log show \
        --last 2m --style compact \
        --predicate 'process == "SpringBoard" AND (eventMessage CONTAINS[c] "status" OR subsystem CONTAINS[c] "StatusBar")'
    } >> "${diag3608}" 2>&1
    [[ "${shot_rc}" -eq 0 ]] || return "${shot_rc}"
  else
    local window_id
    window_id="$("${command_deadline}" run --label "capture:window-id" \
      --deadline-at "${deadline_at}" --receipt "${receipts_file}" \
      --max-seconds "${COMMAND_BOUND_S}" \
      -- "${swift_tools_dir}/window-id" "${pid}")" || return
    rm -f "${target}"
    "${command_deadline}" run --label "capture:screencapture" \
      --deadline-at "${deadline_at}" --receipt "${receipts_file}" \
      --max-seconds "${COMMAND_BOUND_S}" \
      -- screencapture -x -o "-l${window_id}" "${target}" || return
  fi
  if [[ ! -f "${target}" ]]; then
    echo "::error::${example}: the capture command exited 0 but produced no image at ${target}."
    return 1
  fi
}

# Captures until every frame across SETTLE_WINDOW_S seconds agrees with the
# window's first frame within the oracle's default FLIP limits — the app has settled once its
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
# The settle budget is one shared monotonic deadline every supervised command
# in the loop draws down — screenshots, window-id/screencapture, and the
# compare/sleep overhead between them — not a fresh 90 s per attempt.
CAPTURE_DEADLINE_S=90
# An iteration starts only while the budget can still pay for the capture and
# compare it runs. `remaining` reports success down to the last fraction of a
# second, and a command begun on those crumbs is guaranteed to be killed
# mid-flight — iOS starfield in run 38811188166 started a screenshot with
# 0.48 s left, and macOS stress in run 38677209793 a settle compare, each
# reported as a hard capture failure instead of the never-settled outcome
# the same deadline produced everywhere else (#2714). The floor covers one
# measured iteration — simctl screenshot ~0.5 s, window-id+screencapture
# ~1 s, compare ~0.02 s — with an order of magnitude of headroom, the same
# scale as the deadline constants below. A command that starts above it and
# still reaches the deadline ran far past its healthy cost, which stays the
# #1539 stuck-pathology failure.
CAPTURE_ITERATION_FLOOR_S=10
# Native cleanup (terminate/uninstall of the shard-owned app) gets its own
# scoped deadline per call: a healthy simctl answers in under a second but a
# loaded CoreSimulator has answered a benign call in ~30 s, while the stuck
# pathology behind #1539 measured 250 s.
CLEANUP_DEADLINE_S=60
# A LaunchServices openApplication round-trip answers in well under a second
# on a healthy runner; the bound covers a wedged LaunchServices, not the app's
# own startup — the liveness check and first-paint marker still cover that.
LAUNCH_DEADLINE_S=60
capture_settled() {
  local target="$1"
  local pid="${2:-}"
  local anchor="${shots_dir}/.settle-anchor.png"
  local window_start=0
  local deadline_at frame_rc compare_rc remaining_s
  deadline_at="$("${command_deadline}" deadline --seconds "${CAPTURE_DEADLINE_S}")"
  if [[ -f "${baselines_dir}/${platform}/${example}.skip" ]]; then
    # A `.skip` first screen cannot be made deterministic by construction —
    # continuous animation, live media — so the gate asks for exactly one
    # supervised capture after the first-paint marker rather than stability
    # the example declares it cannot produce. Launch and image decoding
    # check downstream still gate; a wedged command still fails on its own
    # bound. The marker applies to the reference twin's capture here too.
    capture_frame "${target}" "${deadline_at}" ${pid:+"${pid}"}
    return
  fi
  rm -f "${anchor}"
  # The sole loop bound is the shared CLOCK_MONOTONIC budget, which the
  # supervised commands themselves draw down; SECONDS is used only for the
  # SETTLE_WINDOW_S agreement duration, as before. The --at-least floor keeps
  # the loop from opening an iteration the budget can no longer pay for —
  # under it the loop takes the deterministic never-settled exit below
  # instead of starting a capture the deadline will kill mid-command.
  while remaining_s="$("${command_deadline}" remaining --deadline-at "${deadline_at}" \
      --at-least "${CAPTURE_ITERATION_FLOOR_S}")"; do
    if capture_frame "${target}" "${deadline_at}" ${pid:+"${pid}"}; then
      if [[ -f "${anchor}" ]]; then
        compare_rc=0
        # Only compare's stdout is noise here; stderr stays — it carries the
        # supervisor's command_started/final receipt JSON and the child's own
        # diagnostics, which is what attributes a hang if the run dies mid-way.
        "${command_deadline}" run --label "capture:compare-screenshots" \
          --deadline-at "${deadline_at}" --receipt "${receipts_file}" \
          --max-seconds "${COMMAND_BOUND_S}" \
          -- "${compare_tool}" \
             compare "${anchor}" "${target}" "${shots_dir}/.settle-diff.png" \
             "${comparison_args[@]}" \
             >/dev/null || compare_rc=$?
        # Only a comparison that ran to completion may steer the anchors:
        # exit 0 = frames agree within FLIP tolerance, exit 1 = a completed
        # over-tolerance comparison restarts the window. Exit 2 (decode/write
        # error) or any other status is a command failure — no anchor
        # update, no earlier-frame substitution. A 124 carries two cases:
        # killed at the scoped bound while shared budget remains is a hung
        # compare — the failure itself; killed at the shared deadline is
        # ordinary budget exhaustion mid-command, which lands on the same
        # never-settled outcome as expiring between polls.
        if (( compare_rc == 0 )); then
          if (( SECONDS - window_start >= SETTLE_WINDOW_S )); then
            rm -f "${anchor}"
            return 0
          fi
        elif (( compare_rc == 1 )); then
          # The screen changed: the window restarts here.
          cp "${target}" "${anchor}"
          window_start=${SECONDS}
        elif (( compare_rc == 124 )) && \
             ! "${command_deadline}" remaining --deadline-at "${deadline_at}" >/dev/null; then
          # The shared deadline spent itself inside the compare — the
          # loop's ordinary exhausted-budget outcome, not a command fault.
          break
        else
          rm -f "${anchor}"
          if (( compare_rc == 124 )); then
            echo "::error::${example}: the settle compare exceeded its ${COMMAND_BOUND_S}s command bound with shared budget remaining — a hung command, not settle progress; last receipt in ${receipts_file}."
          else
            echo "::error::${example}: the settle compare command failed (exit ${compare_rc}); last receipt in ${receipts_file}."
          fi
          return 1
        fi
      else
        # First usable frame: the window starts here.
        cp "${target}" "${anchor}"
        window_start=${SECONDS}
      fi
    else
      # A native capture command that fails is the failure itself:
      # retrying it or falling back to an earlier frame would substitute a
      # state the harness never verified (#1539). The same two 124 cases
      # as the compare apply — a kill at the scoped bound while budget
      # remains is a hung command; a kill at the shared deadline is the
      # loop's ordinary exhaustion, settled below.
      frame_rc=$?
      if (( frame_rc == 124 )) && \
         ! "${command_deadline}" remaining --deadline-at "${deadline_at}" >/dev/null; then
        break
      fi
      rm -f "${anchor}"
      if (( frame_rc == 124 )); then
        echo "::error::${example}: a capture command exceeded its ${COMMAND_BOUND_S}s command bound with shared budget remaining — a hung command, not settle progress; last receipt in ${receipts_file}."
      else
        echo "::error::${example}: a capture command failed (exit ${frame_rc}); last receipt in ${receipts_file}."
      fi
      return 1
    fi
    # The loop-top reading is stale by the time the screenshot and compare
    # have drawn it down, so re-read the shared budget now: expiry at this
    # point is the ordinary exhausted loop — the last successful frame is
    # kept below — and the sleep never runs past the deadline.
    if ! remaining_s="$("${command_deadline}" remaining --deadline-at "${deadline_at}")"; then
      break
    fi
    sleep "$(awk -v r="${remaining_s}" 'BEGIN{print (r<1)?r:1}')"
  done
  rm -f "${anchor}"
  if [[ ! -f "${target}" ]]; then
    return 1
  fi
  echo "::warning::${example} never settled to a stable frame; using the last capture. A first screen that cannot be made deterministic declares a .skip marker."
}

# One supervised `simctl terminate|uninstall <udid> <bundle>` of a shard-owned
# app, bounded by CLEANUP_DEADLINE_S — these calls used to run `|| true`
# unbounded, so a stuck CoreSimulator turned 250 s of silent stall into a
# green cleanup. Observed on the iOS 27.0 runtime this script runs against:
# `terminate` of an app that already exited fails with exit 3 and stderr
# "Simulator device failed to terminate … (domain=NSPOSIXErrorDomain,
# code=3) … found nothing to terminate" — exactly that result, exit 3 with
# the message, is the only benign already-gone outcome. `uninstall` of a
# bundle id that is not installed exits 0 quietly. Any other nonzero status,
# and any deadline kill, is a real failure the shard reports instead of
# hiding.
simctl_cleanup() {
  local what="$1" bundle="$2"
  local deadline_at child_err rc=0
  # The child's stderr goes to an owned per-invocation file — the helper's
  # command_started/receipt records stay on the inherited live stderr, so a
  # stuck cleanup is attributable in the step log instead of vanishing into
  # a captured variable nobody reads until the command settles.
  child_err="$(mktemp "${logs_dir}/.cleanup-${what}-stderr.XXXXXX")"
  deadline_at="$("${command_deadline}" deadline --seconds "${CLEANUP_DEADLINE_S}")"
  "${command_deadline}" run --label "cleanup:simctl-${what}" \
    --deadline-at "${deadline_at}" --receipt "${receipts_file}" \
    --child-stderr-file "${child_err}" \
    -- xcrun simctl "${what}" "${SIMULATOR_UDID}" "${bundle}" \
    >/dev/null || rc=$?
  # The child has settled — reaped or deadline-killed — so its stderr is now
  # safe to classify. Retain the diagnostics for every outcome, then drop
  # only this owned file.
  if [[ -s "${child_err}" ]]; then
    echo "simctl ${what} ${bundle} stderr:" >&2
    sed 's/^/  /' "${child_err}" >&2
  fi
  if (( rc == 0 )); then
    rm -f "${child_err}"
    return 0
  fi
  if (( rc == 124 )); then
    echo "::error::${example}: 'simctl ${what} ${bundle}' was killed at the ${CLEANUP_DEADLINE_S}s cleanup deadline; last receipt in ${receipts_file}."
    collect_simulator_stall_diagnostics "'simctl ${what} ${bundle}' killed at the cleanup deadline"
    rm -f "${child_err}"
    return 1
  fi
  if [[ "${what}" == "terminate" ]] && (( rc == 3 )) && \
     grep -q "domain=NSPOSIXErrorDomain, code=3" "${child_err}" && \
     grep -q "found nothing to terminate" "${child_err}"; then
    echo "::notice::${example}: simctl terminate reports '${bundle}' already exited (exit 3 / NSPOSIXErrorDomain code=3); treating as clean."
    rm -f "${child_err}"
    return 0
  fi
  echo "::error::${example}: 'simctl ${what} ${bundle}' failed (exit ${rc}); stderr retained above."
  rm -f "${child_err}"
  return 1
}

# When a supervised iOS command is killed at its deadline the receipts say
# WHICH command and how long it ran — they cannot say WHY the simulator
# channel wedged. The runs behind #1539 showed the app's first paint, then
# screenshot, terminate and uninstall all timing out: the evidence needed is
# the app's own stack, the host simulator services' state, and the device's
# recent log, collected while the wedge still stands rather than after a
# clean retry. The samples self-limit at 3 s and both `log show` reads run
# under DIAG_DEADLINE_S — a wedged channel must not stall the collection —
# and the output lands in logs_dir, which the e2e-logs artifact bundle
# already ships.
DIAG_DEADLINE_S=30
collect_simulator_stall_diagnostics() {
  local reason="$1"
  local diag="${logs_dir}/${platform}-${example}-stall-diagnostics.log"
  local diag_deadline_at
  diag_deadline_at="$("${command_deadline}" deadline --seconds "${DIAG_DEADLINE_S}")"
  {
    echo "=== simulator stall diagnostics: ${reason} ($(date -u '+%Y-%m-%dT%H:%M:%SZ')) ==="
    echo "--- shard app process (pid ${app_pid:-unset}) ---"
    if [[ -n "${app_pid:-}" ]] && kill -0 "${app_pid}" 2>/dev/null; then
      sample "${app_pid}" 3 1 2>&1 | head -60
    else
      echo "app pid ${app_pid:-unset} is not running"
    fi
    echo "--- host simulator service state ---"
    local svc_pids
    svc_pids="$(pgrep -f "CoreSimulator|SimulatorTrampoline|simctl" 2>/dev/null | tr '\n' ',' | sed 's/,$//')"
    [[ -n "${svc_pids}" ]] && ps -o pid,stat,etime,command -p "${svc_pids}" || true
    echo "--- host simulator service stacks (sample 3s each) ---"
    local svc
    for svc in $(pgrep -f "CoreSimulatorService|SimulatorTrampoline" 2>/dev/null); do
      sample "${svc}" 3 1 2>&1 | sed -n '1,40p'
    done
    echo "--- host log, simulator domain, last 5m ---"
    # `log show` can take tens of seconds on a loaded host — supervised so
    # the collection keeps its own bound even when the log is slow.
    "${command_deadline}" run --label "diag:host-log-show" \
      --deadline-at "${diag_deadline_at}" --receipt "${receipts_file}" \
      -- log show --last 5m --style compact \
      --predicate 'process CONTAINS[c] "simulator" OR subsystem CONTAINS[c] "CoreSimulator"' \
      2>/dev/null | tail -80 || true
    echo "--- device log, last 3m ---"
    "${command_deadline}" run --label "diag:simctl-log-show" \
      --deadline-at "${diag_deadline_at}" --receipt "${receipts_file}" \
      -- xcrun simctl spawn "${SIMULATOR_UDID}" log show --last 3m --style compact \
      2>/dev/null | tail -60 || true
  } >> "${diag}" 2>&1
  echo "::notice::${example}: simulator stall diagnostics written to ${diag} (uploaded with e2e-logs)."
}

# Sends SIGTERM to a LaunchServices-launched pid and waits for it to actually
# disappear: launch-app's pid is a launchd child, not this shell's, so `wait`
# can neither reap nor observe it — `kill -0` is the only liveness signal.
# Bounded by CLEANUP_DEADLINE_S like the iOS cleanup calls: a SIGTERM-ignoring
# app must not hang the shard, and its windows must leave the screen before
# the next capture runs.
terminate_pid() {
  local pid="$1"
  kill "${pid}" 2>/dev/null || true
  local deadline=$(( SECONDS + CLEANUP_DEADLINE_S ))
  while kill -0 "${pid}" 2>/dev/null; do
    (( SECONDS < deadline )) || return 1
    sleep 0.5
  done
}

# ── SwiftUI parity ──────────────────────────────────────────────────────────
# Examples with a registered twin in backends/apple/Tests/E2EReference are also compared
# against the twin rendered live by the reference host on the same runner —
# the backend must stay pixel-faithful to what SwiftUI produces for the same
# layout, not only to its own recorded baseline. The twin registry is read
# from Twins.swift so a new twin joins the sweep automatically.
twin_names=()
if [[ -f "${reference_dir}/Sources/Twins.swift" ]]; then
  # A command substitution for the same reason as discovery above.
  twins="$(sed -n 's/.*case "\([^"]*\)":.*/\1/p' "${reference_dir}/Sources/Twins.swift" | sort -u)"
  while IFS= read -r twin; do
    [[ -n "${twin}" ]] || continue
    twin_names+=("${twin}")
  done <<< "${twins}"
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
  if [[ -n "${PREBUILT_REFERENCE_APP:-}" ]]; then
    local binary="${PREBUILT_REFERENCE_APP}/E2EReference"
    [[ "${platform}" == "macos" ]] && \
      binary="${PREBUILT_REFERENCE_APP}/Contents/MacOS/E2EReference"
    if [[ ! -x "${binary}" ]]; then
      echo "::error::PREBUILT_REFERENCE_APP '${PREBUILT_REFERENCE_APP}' carries no executable ${binary} for platform '${platform}'."
      return 1
    fi
    reference_app="${PREBUILT_REFERENCE_APP}"
    return 0
  fi
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
    simctl_cleanup terminate dev.waterui.E2EReference || rc=1
    return ${rc}
  else
    log stream --predicate 'subsystem == "dev.waterui"' --style compact \
      > "${ref_marker_log}" 2>/dev/null &
    local ref_stream_pid=$!
    sleep 1
    # Launched through LaunchServices so the twin is activated — a direct exec
    # leaves its window non-key. Its stdout/stderr are gone with the direct
    # exec: the twin logs through Logger, which the stream above captures.
    local launch_deadline_at ref_pid
    launch_deadline_at="$("${command_deadline}" deadline --seconds "${LAUNCH_DEADLINE_S}")"
    if ! ref_pid="$("${command_deadline}" run --label "launch:reference" \
        --deadline-at "${launch_deadline_at}" --receipt "${receipts_file}" \
        -- "${swift_tools_dir}/launch-app" "${reference_app}" \
        -E2EExample "${example}" -E2ETitle "${title}")"; then
      kill "${ref_stream_pid}" 2>/dev/null || true
      return 1
    fi
    local rc=0
    if wait_for_reference_first_paint "${ref_marker_log}"; then
      capture_settled "${target}" "${ref_pid}"
      rc=$?
    else
      echo "::error::${example}: the SwiftUI twin never reported a first paint on macos; refusing to compare against an unrendered reference."
      rc=1
    fi
    kill "${ref_stream_pid}" 2>/dev/null || true
    terminate_pid "${ref_pid}" || rc=1
    return ${rc}
  fi
}

# Comparison identity comes from the running OS, not the build SDK.
if [[ "${platform}" == "ios" ]]; then
  comparison_os_major="$(xcrun simctl list devices --json | python3 -c '
import json, re, sys
for runtime, group in json.load(sys.stdin)["devices"].items():
    if any(d.get("udid") == sys.argv[1] for d in group):
        m = re.search(r"iOS-(\d+)", runtime)
        if m:
            print(m.group(1))
        break
' "${SIMULATOR_UDID}")"
else
  comparison_os_major="$(sw_vers -productVersion | cut -d. -f1)"
fi
if [[ -z "${comparison_os_major}" ]]; then
  echo "::error::Cannot derive the running ${platform} major version for screenshot comparison." >&2
  exit 1
fi

# Pixels per point of every capture this job takes, which sizes the twin
# comparison's block in points: the simulated device's screen scale on iOS,
# the runner's single display on macOS.
if [[ "${platform}" == "ios" ]]; then
  device_type_bundle="$(python3 -c '
import json, subprocess, sys
devices = json.loads(subprocess.check_output(["xcrun", "simctl", "list", "devices", "--json"]))
kind = next(d["deviceTypeIdentifier"] for group in devices["devices"].values()
            for d in group if d.get("udid") == sys.argv[1])
types = json.loads(subprocess.check_output(["xcrun", "simctl", "list", "devicetypes", "--json"]))
print(next(t["bundlePath"] for t in types["devicetypes"] if t["identifier"] == kind))
' "${SIMULATOR_UDID}")" || device_type_bundle=""
  capture_scale="$(plutil -extract capabilities.ScreenDimensionsCapability.main-screen-scale raw \
    "${device_type_bundle}/Contents/Resources/capabilities.plist")" || capture_scale=""
  capture_scale="${capture_scale%.000000}"
else
  capture_scale="$("${swift_tools_dir}/display-scale")" || capture_scale=""
fi
if [[ ! "${capture_scale}" =~ ^[1-9][0-9]*$ ]]; then
  echo "::error::Cannot derive the ${platform} capture scale for twin comparison (got '${capture_scale}')." >&2
  exit 1
fi

# Final checks have the same per-command bound and receipt trail as capture.
run_comparison() {
  local label="$1" output="$2"
  shift 2
  local deadline_at
  deadline_at="$("${command_deadline}" deadline --seconds "${COMMAND_BOUND_S}")" || return
  "${command_deadline}" run --label "${label}" --deadline-at "${deadline_at}" \
    --receipt "${receipts_file}" --max-seconds "${COMMAND_BOUND_S}" \
    --child-stderr-file "${output%.json}.log" \
    -- "${compare_tool}" "$@" > "${output}"
}

comparison_summary() {
  python3 -c '
import json, sys
with open(sys.argv[1]) as stream:
    comparison = json.load(stream)
json.dump({key: comparison[key] for key in ("metric", "flip_mean", "flip_max", "max_local_error", "max_block_flip_mean", "limits")},
          sys.stdout)
' "$1"
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
  comparison_args=(--platform "${platform}" --os-major "${comparison_os_major}" --example "${example}")
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
    # The package is backgrounded so the wait loop can bound a hung build by
    # silence rather than duration.
    {
      water package --platform "${package_platform}" --backend apple --release \
        --path "${example_path}" >> "${run_log}" 2>&1 &
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
    }
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

  # Launch the packaged release app. The log stream attaches BEFORE
  # the launch — inside the simulator on iOS, where the app's logd actually
  # lives — so the first-paint marker cannot be lost to an attach race. A
  # missing marker after the grace window is data (the app ran, the marker
  # pipeline broke), not a launch failure.
  stream_pid=""
  app_pid=""
  if [[ "${platform}" == "macos" ]]; then
    log stream --predicate 'subsystem == "dev.waterui"' --style compact \
      > "${marker_log}" 2>/dev/null &
    stream_pid=$!
    sleep 1
    # The app must come up through LaunchServices: a direct exec leaves it
    # unactivated, so its window never becomes key (#2636).
    launch_deadline_at="$("${command_deadline}" deadline --seconds "${LAUNCH_DEADLINE_S}")"
    if ! app_pid="$("${command_deadline}" run --label "launch:app" \
        --deadline-at "${launch_deadline_at}" --receipt "${receipts_file}" \
        -- "${swift_tools_dir}/launch-app" "${app_path}")"; then
      kill "${stream_pid}" 2>/dev/null || true
      echo "::error::${example} packaged app failed to launch (${platform})."
      failures+=("${example}: launch")
      report+=("| \`${example}\` | launch failed | — | — | ${size_cell} | ${mem_cell} |")
      printf '  "%s": null,\n' "${example}" >> "${startup_entries}"
      printf '  "%s": null,\n' "${example}" >> "${memory_entries}"
      echo "::endgroup::"
      continue
    fi
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
    echo "app launch at $(date -u +%Y-%m-%dT%H:%M:%SZ) (override log: ios-${shard_index}-status-bar-override.txt)" \
      >> "${logs_dir}/ios-${example}-status-bar-diag.txt"
    app_pid="$(xcrun simctl launch "${SIMULATOR_UDID}" "${bundle_id}" | awk -F': ' '{print $2}')"
    echo "app launched at $(date -u +%Y-%m-%dT%H:%M:%SZ) pid=${app_pid}" \
      >> "${logs_dir}/ios-${example}-status-bar-diag.txt"
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
    failures+=("${example}: capture")
    if [[ "${platform}" == "ios" ]]; then
      # A capture that never landed on iOS is the channel-stall class #1539
      # tracks: collect app/service stacks and the device log while the
      # wedge still stands, then run the usual bounded cleanup.
      collect_simulator_stall_diagnostics "capture_settled failed for ${example}"
      simctl_cleanup terminate "${bundle_id}" || failures+=("${example}: cleanup terminate")
      simctl_cleanup uninstall "${bundle_id}" || failures+=("${example}: cleanup uninstall")
    else
      terminate_pid "${app_pid}" || failures+=("${example}: cleanup terminate")
    fi
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
    failures+=("${example}: no first paint")
    if [[ "${platform}" == "ios" ]]; then
      simctl_cleanup terminate "${bundle_id}" || failures+=("${example}: cleanup terminate")
      simctl_cleanup uninstall "${bundle_id}" || failures+=("${example}: cleanup uninstall")
    else
      terminate_pid "${app_pid}" || failures+=("${example}: cleanup terminate")
    fi
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
  app_terminated=1
  if [[ "${platform}" == "ios" ]]; then
    simctl_cleanup terminate "${bundle_id}" || {
      app_terminated=0
      failures+=("${example}: cleanup terminate")
    }
    simctl_cleanup uninstall "${bundle_id}" || failures+=("${example}: cleanup uninstall")
  else
    terminate_pid "${app_pid}" || {
      app_terminated=0
      failures+=("${example}: cleanup terminate")
    }
  fi

  if ! run_comparison "${example}:decode" "${shots_dir}/${platform}-${example}-capture.json" \
      check "${shot}"; then
    echo "::error::Cannot decode the captured screenshot for ${example}; see capture log and command receipts."
    failures+=("${example}: screenshot decode")
    report+=("| \`${example}\` | screenshot decode failed | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    echo "::endgroup::"
    continue
  fi

  baseline="${baselines_dir}/${platform}/${example}.png"
  if [[ "${record}" == "1" ]]; then
    cp "${shot}" "${record_dir}/${platform}/${example}.png"
    report+=("| \`${example}\` | recorded — VISUAL REVIEW REQUIRED | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
  elif [[ -f "${baselines_dir}/${platform}/${example}.skip" ]]; then
    report+=("| \`${example}\` | captured (.skip) — VISUAL REVIEW REQUIRED | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
  elif [[ -f "${baselines_dir}/${platform}/${example}.visual" ]]; then
    # `.visual` — output that is not deterministic by construction
    # (GPU-rendered). The capture pair
    # (WaterUI + SwiftUI twin below) ships as artifacts for human review.
    report+=("| \`${example}\` | launched — VISUAL REVIEW REQUIRED | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
  elif [[ ! -f "${baseline}" ]]; then
    report+=("| \`${example}\` | no baseline — VISUAL REVIEW REQUIRED | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
  else
    diff_image="${shots_dir}/${platform}-${example}-diff.png"
    baseline_json="${shots_dir}/${platform}-${example}-baseline.json"
    compare_rc=0
    run_comparison "${example}:baseline" "${baseline_json}" \
      compare "${baseline}" "${shot}" "${diff_image}" "${comparison_args[@]}" || compare_rc=$?
    compare_out=""
    if (( compare_rc == 0 || compare_rc == 1 )); then
      compare_out="$(comparison_summary "${baseline_json}")"
    fi
    if (( compare_rc == 0 )); then
      report+=("| \`${example}\` | baseline within FLIP tolerance — REVIEW REQUIRED | ${compare_out} | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    elif (( compare_rc == 1 )); then
      echo "::error::Baseline FLIP tolerance exceeded for ${example}: ${compare_out}; inspect the full capture and heatmap."
      failures+=("${example}: baseline FLIP tolerance")
      report+=("| \`${example}\` | baseline exceeds FLIP tolerance — REVIEW REQUIRED | ${compare_out} | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    else
      echo "::error::Baseline comparison command failed for ${example} (exit ${compare_rc}); see its log and command receipts."
      failures+=("${example}: baseline comparison command")
      report+=("| \`${example}\` | baseline comparison failed (${compare_rc}) | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    fi
  fi

  # SwiftUI parity: render the twin in the reference host and compare it
  # against the example capture taken above. A `.parity-skip` marker next to
  # the baselines opts a twin out while a known divergence is worked down.
  # A failed terminate leaves the example potentially frontmost, and a
  # reference launch over it would photograph the "back to <app>" breadcrumb
  # — a corrupted input, not a parity measurement. That prerequisite failure
  # is recorded explicitly for every eligible twin; the coverage does not
  # silently disappear.
  if has_twin "${example}" && \
     [[ ! -f "${baselines_dir}/${platform}/${example}.parity-skip" ]]; then
    if (( ! app_terminated )); then
      echo "::error::${example}: parity prerequisite failed — the example did not terminate, so the reference cannot launch to a clean frontmost state; twin capture not run."
      failures+=("${example}: parity prerequisite")
      report+=("| \`${example}\` (parity) | prerequisite failed (terminate) | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
      echo "::endgroup::"
      continue
    fi
    ref_shot="${shots_dir}/${platform}-${example}-ref.png"
    parity_diff="${shots_dir}/${platform}-${example}-parity-diff.png"
    parity_json="${shots_dir}/${platform}-${example}-parity.json"
    parity_record_flag=""
    [[ "${record}" == "1" ]] && parity_record_flag="--record"
    if ! capture_reference "${ref_shot}" "${example}" "${product}"; then
      echo "::error::Could not capture the SwiftUI reference for ${example}."
      failures+=("${example}: reference capture")
      report+=("| \`${example}\` (parity) | reference failed | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    elif ! run_comparison "${example}:reference-decode" \
        "${shots_dir}/${platform}-${example}-reference.json" check "${ref_shot}"; then
      echo "::error::Cannot decode the SwiftUI reference for ${example}; see reference log and command receipts."
      failures+=("${example}: reference decode")
      report+=("| \`${example}\` (parity) | reference decode failed | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    elif [[ -f "${baselines_dir}/${platform}/${example}.visual" ]]; then
      # `.visual`: both halves of the pair are captured and shipped as
      # artifacts; a human reviews the whole images.
      report+=("| \`${example}\` (parity) | VISUAL REVIEW REQUIRED | pair in e2e-shots artifacts | ${fp_cell} | ${size_cell} | ${mem_cell} |")
    else
      # A twin is a second implementation, not a second route to the same
      # pixels: it is gated by the comparer's twin FLIP limits, which accept
      # edge offsets of up to 2 px and reject missing, extra or misplaced
      # elements, judged over 8 pt blocks at the capture scale.
      parity_rc=0
      run_comparison "${example}:parity" "${parity_json}" \
        compare "${ref_shot}" "${shot}" "${parity_diff}" "${comparison_args[@]}" \
        --limits twin --scale "${capture_scale}" ${parity_record_flag:+"${parity_record_flag}"} || parity_rc=$?
      parity_out=""
      if (( parity_rc == 0 || parity_rc == 1 )); then
        parity_out="$(comparison_summary "${parity_json}")"
      fi
      if (( parity_rc == 0 )); then
        if [[ "${record}" == "1" ]]; then
          mkdir -p "${record_dir}/parity"
          cp "${parity_json}" "${record_dir}/parity/${platform}-${example}.json"
          report+=("| \`${example}\` (parity) | recorded — VISUAL REVIEW REQUIRED | ${parity_out} | ${fp_cell} | ${size_cell} | ${mem_cell} |")
        else
          report+=("| \`${example}\` (parity) | within FLIP tolerance — REVIEW REQUIRED | ${parity_out} | ${fp_cell} | ${size_cell} | ${mem_cell} |")
        fi
      elif (( parity_rc == 1 )); then
        echo "::error::SwiftUI parity FLIP tolerance exceeded for ${example}: ${parity_out}; inspect the full pair and heatmap."
        failures+=("${example}: parity FLIP tolerance")
        report+=("| \`${example}\` (parity) | exceeds FLIP tolerance — REVIEW REQUIRED | ${parity_out} | ${fp_cell} | ${size_cell} | ${mem_cell} |")
      else
        echo "::error::SwiftUI parity comparison command failed for ${example} (exit ${parity_rc}); see its log and command receipts."
        failures+=("${example}: parity comparison command")
        report+=("| \`${example}\` (parity) | comparison failed (${parity_rc}) | — | ${fp_cell} | ${size_cell} | ${mem_cell} |")
      fi
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
