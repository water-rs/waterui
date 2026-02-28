#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
IMAGE="waterui-hydrolysis-wayland-smoke:latest"

cat >"${ROOT_DIR}/backends/hydrolysis/scripts/.Dockerfile.wayland-smoke" <<'DOCKERFILE'
FROM rust:1.88-bookworm

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    weston \
    libwayland-dev \
    libxkbcommon-dev \
    libegl1-mesa \
    libgles2-mesa \
    libvulkan1 \
    mesa-vulkan-drivers \
    libudev-dev \
    libasound2-dev \
    pkg-config \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /work
DOCKERFILE

cleanup() {
    rm -f "${ROOT_DIR}/backends/hydrolysis/scripts/.Dockerfile.wayland-smoke"
}
trap cleanup EXIT

docker build -f "${ROOT_DIR}/backends/hydrolysis/scripts/.Dockerfile.wayland-smoke" -t "${IMAGE}" "${ROOT_DIR}"

docker run --rm -t \
    -v "${ROOT_DIR}:/work" \
    -w /work \
    "${IMAGE}" \
    bash -lc '
        set -euo pipefail
        export XDG_RUNTIME_DIR=/tmp/xdg-runtime
        mkdir -p "$XDG_RUNTIME_DIR"
        chmod 700 "$XDG_RUNTIME_DIR"

        weston \
            --backend=headless-backend.so \
            --socket=wayland-1 \
            --idle-time=0 \
            --log=/tmp/weston.log &
        WESTON_PID=$!

        for _ in $(seq 1 80); do
            if [ -S "$XDG_RUNTIME_DIR/wayland-1" ]; then
                break
            fi
            sleep 0.1
        done

        if [ ! -S "$XDG_RUNTIME_DIR/wayland-1" ]; then
            echo "weston did not create wayland-1 socket" >&2
            cat /tmp/weston.log >&2 || true
            kill "$WESTON_PID" || true
            exit 1
        fi

        export WAYLAND_DISPLAY=wayland-1
        export WINIT_UNIX_BACKEND=wayland

        cargo run -p hydrolysis --features winit --example wayland_smoke

        kill "$WESTON_PID" || true
        wait "$WESTON_PID" || true
    '
