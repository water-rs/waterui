#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/.." && pwd )"

# Xcode runs build scripts in a restricted shell environment without the user's
# full PATH. Add common tool locations explicitly.
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

if ! command -v water >/dev/null 2>&1; then
    echo "error: the 'water' CLI is not on PATH. Install it with 'cargo install waterui-cli'." >&2
    echo "error: Searched in PATH: $PATH" >&2
    exit 1
fi

CONFIGURATION_VALUE="${CONFIGURATION:-Debug}"
set -- build apple --project "$PROJECT_ROOT"
if [ "$CONFIGURATION_VALUE" = "Release" ]; then
    set -- "$@" --release
fi

exec water "$@"
