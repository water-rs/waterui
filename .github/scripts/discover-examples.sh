#!/usr/bin/env bash
# Prints the runnable example names (directories holding a Cargo.toml under
# examples/ and backends/apple/Examples/), sorted and unique, one per line;
# fails when there are none.
#
# Requires bash >= 4.2, asserted below (#2683): /bin/bash 3.2 installs its
# SIGCHLD handler without SA_RESTART (bash-4.2 fixed "an arriving SIGCHLD
# interrupting output functions like those invoked by echo or printf"), so
# under 3.2 a child exiting while a builtin writes can fail the write with
# EINTR. Discovery still runs in one command substitution — it keeps the
# exit status (a captured `::error` line would otherwise become an example
# name) and the shell waits the child out before printing anything.
set -euo pipefail

if (( BASH_VERSINFO[0] < 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] < 2) )); then
  echo "::error::discover-examples.sh needs bash >= 4.2 (found ${BASH_VERSION}); /bin/bash 3.2 lets SIGCHLD interrupt writes with EINTR (#2683)" >&2
  exit 1
fi

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
