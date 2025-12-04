#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

if ! command -v water >/dev/null 2>&1; then
    echo "error: the 'water' CLI is not on PATH. Install it with 'cargo install waterui-cli'." >&2
    exit 1
fi

# Usage: build-rust.sh <target-triple> [release]
# Example: build-rust.sh aarch64-linux-android release
TARGET="${1:-}"
PROFILE="${2:-debug}"

if [ -z "$TARGET" ]; then
    echo "error: target triple is required" >&2
    echo "usage: build-rust.sh <target-triple> [release]" >&2
    echo "example: build-rust.sh aarch64-linux-android release" >&2
    exit 1
fi

CLI_ARGS=(build "$TARGET" --project "$SCRIPT_DIR")
if [ "$PROFILE" = "release" ]; then
    CLI_ARGS+=(--release)
fi

exec water "${CLI_ARGS[@]}"
