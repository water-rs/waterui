#!/usr/bin/env bash
# Nightly release-metrics job. Builds a minimal hello-world playground against
# the framework checkout itself (this repository — the backend under test is
# its in-tree `backends/apple`), packages it in release mode for each platform, and
# records three release metrics per platform in release-metrics/release-metrics.json:
#
#   app_bytes / executable_bytes - packaged bundle and main-binary size, gated
#     against Tests/E2EBaselines/package-size.json (5% tolerance fails the job)
#   first_paint_ms - process start to first frame, proven by the backend's
#     waterui_first_paint_ms os_log marker arriving on the launched app's own
#     pid (measure-native-launch.py)
#   peak_rss_bytes - peak resident set of that same owned pid, sampled over a
#     post-launch idle window by the same helper
# Every measured value is bound to the artifact the invoked CLI reported and
# to the launch this run created; records are JSON lines throughout, and a
# record run (RECORD=1) writes fresh size baselines for the publish-baselines
# flow. Startup and memory are recorded, not gated.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
waterui_dir="${WATERUI_DIR:?WATERUI_DIR must point at the prepared waterui checkout}"
repo_root="${GITHUB_WORKSPACE:-$(pwd)}"
baseline_file="${repo_root}/backends/apple/Tests/E2EBaselines/package-size.json"
record_dir="${RECORD_DIR:-${repo_root}/e2e-baselines}"
record="${RECORD:-0}"

# Resolve the CLI under test explicitly: WATER_BIN wins over PATH, and the
# resolved path plus its version land in the log, so an older install (e.g.
# `~/.cargo/bin/water` shadowing the freshly built artifact) is visible
# rather than silently driving the measurements.
water_bin="${WATER_BIN:-$(command -v water || true)}"
if [[ -z "${water_bin}" || ! -x "${water_bin}" ]]; then
    echo "::error::No water CLI on PATH; set WATER_BIN to the build under test"
    exit 1
fi
water_bin="$(cd "$(dirname "${water_bin}")" && pwd)/$(basename "${water_bin}")"
echo "check-package-size: water CLI is ${water_bin}"
"${water_bin}" --version || {
    echo "::error::${water_bin} could not report its version"
    exit 1
}

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/waterui-size-check.XXXXXX")"
trap 'rm -rf "${work_dir}"' EXIT
project_dir="${work_dir}/helloworld"
mkdir -p "${project_dir}/src"
measured_jsonl="${work_dir}/measured.jsonl"
runtime_jsonl="${work_dir}/runtime.jsonl"

# The generated project resolves the backend through waterui_path's
# canonical backends/apple slot — this checkout — so the packaged app builds
# the commit under test, not a pinned release.
# Water.toml has no [backends.*] table for this: a local runtime checkout is
# discovered at waterui_path/backends/apple, never declared.
cat > "${project_dir}/Water.toml" <<EOF
waterui_path = "${waterui_dir}"

[package]
name = "Hello World"
bundle_identifier = "com.waterui.helloworld"
EOF

cat > "${project_dir}/Cargo.toml" <<EOF
[package]
name = "helloworld"
version = "0.1.0"
edition = "2024"
publish = false

[features]
dev = ["waterui/dynamic_linking"]

[dependencies]
waterui = { path = "${waterui_dir}" }
EOF

cat > "${project_dir}/src/lib.rs" <<'EOF'
use waterui::app::App;
use waterui::prelude::*;

fn hello() -> impl View {
    text("Hello, World!")
}

pub fn app(env: Environment) -> App {
    App::new(hello, env)
}
EOF

# Extract one field from a JSON record without lossy whitespace splitting —
# paths and identifiers arrive verbatim.
json_field() {
    python3 -c 'import json, sys; print(json.loads(sys.argv[1])[sys.argv[2]])' "$1" "$2"
}

measure_platform() {
    local label="$1" platform="$2" subject_dir="$3"
    local log_file="${work_dir}/package-${label}-${platform}.log"

    echo "=== Packaging ${label} for ${platform} (release)"
    if ! "${water_bin}" package --platform "${platform}" --backend apple --release --path "${subject_dir}" \
        > "${log_file}" 2>&1; then
        tail -40 "${log_file}" || true
        echo "::error::water package failed for ${label} on ${platform}; see output above"
        return 1
    fi

    # The only artifact this run may measure is the one the invoked CLI
    # reports. `water package` emits `Packaged at <path>` once the bundle has
    # landed in the project's target/package directory; a missing or
    # malformed report fails the platform. There is deliberately no search
    # of shared build caches: the first .app found there can be another
    # subject's bundle or an older run's, and measuring it would attribute
    # another artifact's size to this commit.
    local app_path
    app_path="$(sed -n 's/.*Packaged at //p' "${log_file}" | tail -1 | sed 's/\x1b\[[0-9;]*m//g' | tr -d '\r')"
    if [[ -z "${app_path}" || ! -d "${app_path}" ]]; then
        echo "::error::water package did not report a packaged bundle for ${label} on ${platform}"
        return 1
    fi
    if ! "${script_dir}/release-metrics.py" inspect-app \
            --label "${label}" --platform "${platform}" \
            --app "${app_path}" --out "${measured_jsonl}"; then
        echo "::error::reported bundle is malformed for ${label} on ${platform}: ${app_path}"
        return 1
    fi
}

# Runtime metrics come from one launch measure-native-launch.py owns end to
# end: it attaches the dev.waterui log stream and waits for the stream's
# attach header BEFORE launching (no fixed sleep can prove that), accepts
# the first-paint marker only from this launch's pid, samples the owned
# pid's RSS over the post-launch idle window, and terminates the app on
# every path. A launch that cannot be proven fails the platform and records
# nulls; a launched app whose marker never arrives records a null
# first_paint_ms — the app ran, the marker pipeline broke, and that
# distinction belongs in the data.
measure_runtime() {
    local label="$1" platform="$2" app_path="$3" executable="$4" bundle_id="$5"
    local report="${work_dir}/runtime-${label}-${platform}.json"
    local rc=0

    if [[ "${platform}" == macos ]]; then
        "${script_dir}/measure-native-launch.py" macos "${executable}" \
            --metrics-json "${report}" || rc=$?
    else
        local udid="${SIMULATOR_UDID:?SIMULATOR_UDID is required for ios-simulator runtime metrics}"
        if ! xcrun simctl install "${udid}" "${app_path}"; then
            echo "::error::${label}/${platform} release app failed to install in the simulator"
            rc=1
        else
            "${script_dir}/measure-native-launch.py" ios-simulator "${udid}" "${bundle_id}" \
                --metrics-json "${report}" || rc=$?
        fi
        xcrun simctl uninstall "${udid}" "${bundle_id}" > /dev/null 2>&1 || true
    fi

    if [[ "${rc}" != 0 ]]; then
        echo "::error::${label}/${platform} release app could not be launched for measurement"
        "${script_dir}/release-metrics.py" record-runtime \
            --label "${label}" --platform "${platform}" --out "${runtime_jsonl}"
        return 1
    fi
    "${script_dir}/release-metrics.py" record-runtime \
        --label "${label}" --platform "${platform}" \
        --report "${report}" --out "${runtime_jsonl}"
}

# Measurement subject: the generated hello-world — the stable minimal-app
# signal the size gate applies to. Per-example release sizes/startup/memory
# are measured by the e2e shards themselves, which package every example in
# release mode; duplicating example subjects here would pay the packaging
# cost twice (#144).
subjects=("helloworld=${project_dir}")

failed=0
for entry in ${subjects[@]+"${subjects[@]}"}; do
    label="${entry%%=*}"
    subject_dir="${entry#*=}"
    for platform in ${PLATFORMS:-macos ios-simulator}; do
        measure_platform "${label}" "${platform}" "${subject_dir}" || failed=1
    done
done
[[ -f "${measured_jsonl}" ]] || { echo "::error::No platform packaged successfully"; exit 1; }

# Runtime metrics on the freshly packaged release builds. iOS needs a booted
# simulator; when SIMULATOR_UDID is unset the size gate still runs and the
# ios-simulator row records nulls rather than failing the job.
while IFS= read -r row <&3; do
    label="$(json_field "${row}" label)"
    platform="$(json_field "${row}" platform)"
    if [[ "${platform}" != macos && -z "${SIMULATOR_UDID:-}" ]]; then
        echo "::warning::No booted simulator for ${label}/${platform}; runtime metrics skipped"
        "${script_dir}/release-metrics.py" record-runtime \
            --label "${label}" --platform "${platform}" --out "${runtime_jsonl}"
        continue
    fi
    measure_runtime "${label}" "${platform}" \
        "$(json_field "${row}" app_path)" \
        "$(json_field "${row}" executable)" \
        "$(json_field "${row}" bundle_id)" || failed=1
done 3< "${measured_jsonl}"

# Machine-readable record + human-readable table, then the record or gate
# leg: RECORD=1 writes fresh size baselines; otherwise the hello-world byte
# sizes are held to their recorded baseline within 5%. A missing baseline
# reports without gating.
metrics_dir="${METRICS_DIR:-${repo_root}/release-metrics}"
report_args=(
    --measured "${measured_jsonl}"
    --runtime "${runtime_jsonl}"
    --metrics-dir "${metrics_dir}"
)
if [[ "${record}" == "1" ]]; then
    report_args+=(--record "${record_dir}/package-size.json")
else
    report_args+=(--baseline "${baseline_file}")
fi
"${script_dir}/release-metrics.py" report "${report_args[@]}" || failed=1

if [[ "${failed}" == 1 ]]; then
    echo "Release metrics failed: see the errors above — a package that did not build, a runtime measurement that did not complete, or a size past its baseline (refresh the baseline only if the growth is intended: record_baselines run)."
    exit 1
fi
if [[ "${record}" == 1 ]]; then
    echo "Baselines recorded."
else
    echo "Package sizes within baseline."
fi
