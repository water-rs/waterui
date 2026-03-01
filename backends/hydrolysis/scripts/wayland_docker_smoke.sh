#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
IMAGE="waterui-hydrolysis-wayland-smoke:latest"
DOCKERFILE="${ROOT_DIR}/backends/hydrolysis/scripts/wayland-smoke.Dockerfile"
INNER_SCRIPT="/work/backends/hydrolysis/scripts/wayland_docker_smoke_inner.sh"

docker build -f "${DOCKERFILE}" -t "${IMAGE}" "${ROOT_DIR}"

docker run --rm -t \
    -v "${ROOT_DIR}:/work" \
    -w /work \
    "${IMAGE}" \
    bash "${INNER_SCRIPT}"
