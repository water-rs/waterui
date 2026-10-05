#!/usr/bin/env bash
# Container entrypoint: prepares cgroup delegation (the kernel's
# no-internal-process rule blocks +memory until root procs move out), a
# writable XDG_RUNTIME_DIR, then runs the requested command.
set -e
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/bench-xdg}"
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
mkdir -p /sys/fs/cgroup/init 2>/dev/null || true
while read -r p; do echo "$p" > /sys/fs/cgroup/init/cgroup.procs 2>/dev/null || true; done < /sys/fs/cgroup/cgroup.procs
exec "$@"
