#!/usr/bin/env bash
# Prints the runnable example names (directories holding a Cargo.toml under
# examples/ and backends/apple/Examples/), sorted and unique, one per line;
# fails when there are none.
#
# The macOS runners execute this with /bin/bash 3.2, which installs its
# SIGCHLD handler without SA_RESTART (bash-4.2 fixed "an arriving SIGCHLD
# interrupting output functions like those invoked by echo or printf"). A
# child exiting while a builtin writes therefore fails the write with EINTR.
# So the shell never writes while a child of its own is alive: discovery runs
# in one command substitution, which the shell waits out to the last process
# before it prints anything — no process substitution, no background child.
set -euo pipefail

waterui_dir="${1:-${WATERUI_DIR:-${GITHUB_WORKSPACE:-$(pwd)}}}"
examples_roots=("${waterui_dir}/examples" "${waterui_dir}/backends/apple/Examples")

# Sort + unique: a name present in both roots is one example entry (the
# consumers resolve root examples/ first, then backends/apple/Examples/).
examples="$(
  for examples_root in "${examples_roots[@]}"; do
    [[ -d "${examples_root}" ]] || continue
    find "${examples_root}" -mindepth 1 -maxdepth 1 -type d -print0 |
      while IFS= read -r -d '' example_dir; do
        if [[ -f "${example_dir}/Cargo.toml" ]]; then
          printf '%s\n' "${example_dir##*/}"
        fi
      done
  done | sort -u
)"

if [[ -z "${examples}" ]]; then
  echo "::error::No runnable examples found under ${examples_roots[*]}"
  exit 1
fi

printf '%s\n' "${examples}"
