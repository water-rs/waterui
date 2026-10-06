#!/usr/bin/env bash
# Container entrypoint: a writable XDG_RUNTIME_DIR, then the requested
# command. The measurement cgroup (the move out of the root cgroup and
# +memory delegation) is benchcomp's own setup, which fails the rep when
# it cannot be done; build containers never touch cgroups.
set -euo pipefail
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/bench-xdg}"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
exec "$@"
