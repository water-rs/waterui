#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

if ! command -v water >/dev/null 2>&1; then
    echo "error: the 'water' CLI is not on PATH. Install it with 'cargo install waterui-cli'." >&2
    exit 1
fi

CONFIGURATION_VALUE="${CONFIGURATION:-Debug}"
set -- build apple --project "$PROJECT_ROOT"
if [ "$CONFIGURATION_VALUE" = "Release" ]; then
    set -- "$@" --release
fi

exec water "$@"
