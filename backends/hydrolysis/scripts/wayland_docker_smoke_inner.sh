#!/usr/bin/env bash
set -euo pipefail

SCREENSHOT="/work/backends/hydrolysis/scripts/wayland_smoke.png"
SCREENSHOT_DIR="/work/backends/hydrolysis/scripts"
APP_LOG="/work/backends/hydrolysis/scripts/wayland_smoke_app.log"
WESTON_LOG_OUT="/work/backends/hydrolysis/scripts/wayland_smoke_weston.log"
SCREENSHOT_LOG_OUT="/work/backends/hydrolysis/scripts/wayland_smoke_screenshooter.log"
WESTON_LOG="/tmp/weston.log"
SCREENSHOT_LOG="/tmp/weston-screenshot.log"
BASELINE_SCREENSHOT="/work/backends/hydrolysis/scripts/wayland_smoke_baseline.png"
PROBE_SCREENSHOT="/work/backends/hydrolysis/scripts/wayland_smoke_probe.png"
CANDIDATE_SCREENSHOT="/work/backends/hydrolysis/scripts/wayland_smoke_candidate.png"

persist_logs() {
    cp "${WESTON_LOG}" "${WESTON_LOG_OUT}" 2>/dev/null || true
    cp "${SCREENSHOT_LOG}" "${SCREENSHOT_LOG_OUT}" 2>/dev/null || true
}

cleanup() {
    persist_logs
    rm -f "${BASELINE_SCREENSHOT}" "${PROBE_SCREENSHOT}" "${CANDIDATE_SCREENSHOT}" \
        "${SCREENSHOT_DIR}"/wayland-screenshot-*.png
    if [[ -n "${APP_PID:-}" ]]; then
        kill "${APP_PID}" 2>/dev/null || true
        wait "${APP_PID}" 2>/dev/null || true
    fi
    if [[ -n "${WESTON_PID:-}" ]]; then
        kill "${WESTON_PID}" 2>/dev/null || true
        wait "${WESTON_PID}" 2>/dev/null || true
    fi
}
trap cleanup EXIT

export XDG_RUNTIME_DIR=/tmp/xdg-runtime
mkdir -p "${XDG_RUNTIME_DIR}"
chmod 700 "${XDG_RUNTIME_DIR}"

weston \
    --backend=headless-backend.so \
    --socket=wayland-1 \
    --idle-time=0 \
    --renderer=pixman \
    --modules=screen-share.so \
    --debug \
    --log="${WESTON_LOG}" &
WESTON_PID=$!

for _ in $(seq 1 120); do
    if [ -S "${XDG_RUNTIME_DIR}/wayland-1" ]; then
        break
    fi
    sleep 0.1
done

if [ ! -S "${XDG_RUNTIME_DIR}/wayland-1" ]; then
    echo "weston did not create wayland-1 socket" >&2
    cat "${WESTON_LOG}" >&2 || true
    exit 1
fi

export WAYLAND_DISPLAY=wayland-1
export WINIT_UNIX_BACKEND=wayland
export WGPU_BACKEND=vulkan,gles
export HYDROLYSIS_WAYLAND_SMOKE_SECONDS=15

/usr/local/cargo/bin/cargo build -p hydrolysis --features winit --example wayland_smoke

capture_screenshot() {
    local output_path="$1"
    rm -f "${SCREENSHOT_DIR}"/wayland-screenshot-*.png
    if ! timeout 0.5s bash -lc "cd '${SCREENSHOT_DIR}' && weston-screenshooter" \
        >"${SCREENSHOT_LOG}" 2>&1; then
        return 1
    fi
    local latest
    latest="$(ls -1t "${SCREENSHOT_DIR}"/wayland-screenshot-*.png 2>/dev/null | head -n 1 || true)"
    if [ -z "${latest}" ] || [ ! -s "${latest}" ]; then
        return 1
    fi
    mv "${latest}" "${output_path}"
}

rm -f "${SCREENSHOT}" "${BASELINE_SCREENSHOT}" "${PROBE_SCREENSHOT}" "${CANDIDATE_SCREENSHOT}"
for _ in $(seq 1 120); do
    if capture_screenshot "${BASELINE_SCREENSHOT}"; then
        break
    fi
    sleep 0.1
done
if [ ! -s "${BASELINE_SCREENSHOT}" ]; then
    echo "failed to capture baseline wayland screenshot" >&2
    cat "${SCREENSHOT_LOG}" >&2 || true
    cat "${WESTON_LOG}" >&2 || true
    exit 1
fi

for _ in $(seq 1 120); do
    if capture_screenshot "${PROBE_SCREENSHOT}" && cmp -s "${BASELINE_SCREENSHOT}" "${PROBE_SCREENSHOT}"; then
        rm -f "${PROBE_SCREENSHOT}"
        break
    fi
    if [ -s "${PROBE_SCREENSHOT}" ]; then
        mv "${PROBE_SCREENSHOT}" "${BASELINE_SCREENSHOT}"
    fi
    sleep 0.1
done

/work/target/debug/examples/wayland_smoke >"${APP_LOG}" 2>&1 &
APP_PID=$!

for _ in $(seq 1 120); do
    if ! kill -0 "${APP_PID}" 2>/dev/null; then
        if wait "${APP_PID}"; then
            app_status=0
        else
            app_status=$?
        fi
        echo "wayland_smoke exited before screenshot capture" >&2
        echo "wayland_smoke exit status: ${app_status}" >&2
        cat "${APP_LOG}" >&2 || true
        cat "${WESTON_LOG}" >&2 || true
        exit 1
    fi
    if capture_screenshot "${CANDIDATE_SCREENSHOT}" \
        && ! cmp -s "${BASELINE_SCREENSHOT}" "${CANDIDATE_SCREENSHOT}"; then
        mv "${CANDIDATE_SCREENSHOT}" "${SCREENSHOT}"
        break
    fi
    sleep 0.1
done

if [ ! -s "${SCREENSHOT}" ]; then
    echo "failed to capture wayland screenshot" >&2
    cat "${APP_LOG}" >&2 || true
    cat "${SCREENSHOT_LOG}" >&2 || true
    cat "${WESTON_LOG}" >&2 || true
    exit 1
fi

wait "${APP_PID}"
app_status=$?
if [ "${app_status}" -ne 0 ]; then
    echo "wayland_smoke exited with failure status: ${app_status}" >&2
    cat "${APP_LOG}" >&2 || true
    exit "${app_status}"
fi
echo "wayland screenshot: ${SCREENSHOT}"
