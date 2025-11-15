#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

if ! command -v water >/dev/null 2>&1; then
    echo "error: the 'water' CLI is not on PATH. Install it with 'cargo install waterui-cli'." >&2
    exit 1
fi

PROFILE=${1:-debug}
CLI_ARGS=()
if [ "$PROFILE" = "release" ]; then
    CLI_ARGS+=(--release)
fi

if [ "${#CLI_ARGS[@]}" -gt 0 ]; then
    exec water build android --project "$SCRIPT_DIR" "${CLI_ARGS[@]}"
else
    exec water build android --project "$SCRIPT_DIR"
fi
