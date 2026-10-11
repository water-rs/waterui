#!/usr/bin/env bash
# Warm the e2e workspace for Devin sessions (#287): keep the checkouts an
# e2e run needs under ~/repos, build the `water` CLI once, and
# `water package --release` each example for one platform so the shared
# cargo target cache and sccache are hot before a session starts. The Apple
# backend is in-tree at waterui/backends/apple — no staging step. Sessions
# then fetch the heads under test into these same checkouts and each example
# rebuilds only what those heads changed.
#
# usage: warm-e2e-workspace.sh <platform> [example ...]
#   platform   macos or ios — one platform per invocation so each blueprint
#              step stays under the step-time limit
#   example    restrict to these examples; none means every example
#              discover-examples.sh returns for the waterui checkout
set -euo pipefail

# The whole body lives in a function: bash parses a function definition in
# full before executing any of it, so by the time `main "$@"` below runs the
# entire file has already been read. This matters because sync_repo below can
# `git checkout` the very checkout this script is executing from (the
# blueprint invokes ~/repos/apple-backend/.github/scripts/warm-e2e-workspace.sh)
# and replace this file mid-run — a top-level script is read incrementally
# and would execute garbage after the replacement.
main() {
  local platform="${1:?usage: warm-e2e-workspace.sh <platform> [example ...]}"
  shift

  case "${platform}" in
    macos|ios) ;;
    *)
      echo "error: unsupported platform '${platform}'. Expected macos or ios." >&2
      exit 1
      ;;
  esac

  local repos_dir="${HOME}/repos"
  local waterui_dir="${repos_dir}/waterui"
  local script_dir
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  # The repo the scripts live in (waterui — the backend is in-tree);
  # package-examples.sh resolves its sibling discovery script relative to
  # the workspace it is given.
  local repo_root
  repo_root="$(cd "${script_dir}/../.." && pwd)"

  mkdir -p "${repos_dir}"

  sync_repo "${waterui_dir}" https://github.com/water-rs/waterui.git
  sync_repo "${repos_dir}/cocoa-ui" https://github.com/water-rs/cocoa-ui.git

  # `water --version` does not expose the build commit, so there is nothing to
  # skip against — cargo install is incremental and a same-source install is
  # near-instant anyway. --force lets the install overwrite a water binary the
  # VM image shipped without a cargo-install record. sccache wraps rustc so
  # dependencies compile once per VM however many checkouts build them.
  export RUSTC_WRAPPER="${RUSTC_WRAPPER:-sccache}"
  export PATH="${HOME}/.cargo/bin:${PATH}"
  cargo install --path "${waterui_dir}/cli" --locked --force

  local -a examples=("$@")
  if (( ${#examples[@]} == 0 )); then
    # bash 3.2 (macOS /bin/bash) has no mapfile; read line-wise instead.
    local example
    while IFS= read -r example; do
      examples+=("${example}")
    done < <("${repo_root}/.github/scripts/discover-examples.sh" "${waterui_dir}")
  fi

  # Reuse package-examples.sh for the `water package` invocation rather than
  # duplicating it; its artifact staging lands in a scratch dir. One example
  # per call gives per-example wall-clock seconds and fails fast — a nonzero
  # exit here aborts the whole run.
  # Global, not local: the EXIT trap still runs after main returns, when a
  # function-local would be out of scope under `set -u`.
  package_scratch="$(mktemp -d)"
  trap 'rm -rf "${package_scratch}"' EXIT
  export GITHUB_WORKSPACE="${repo_root}" \
    WATERUI_DIR="${waterui_dir}" \
    PLATFORM="${platform}" \
    EXAMPLE_LOG_DIR="${package_scratch}/logs" \
    PACKAGE_OUT_DIR="${package_scratch}/packaged"

  local start
  for example in "${examples[@]}"; do
    start=${SECONDS}
    EXAMPLES="${example}" "${repo_root}/.github/scripts/package-examples.sh"
    printf 'example %-20s %ds\n' "${example}" "$((SECONDS - start))"
  done
}

# Sync one support checkout. A checkout that already contains origin/dev is a
# head under test — a session's fetched branch or a commit on top of the
# baseline — and is kept so the warm build exercises exactly that head;
# anything else (stale, diverged, or a fresh clone's default branch) is reset
# to the baseline. Dirty checkouts fail loudly rather than losing work.
sync_repo() {
  local dir="$1" url="$2"
  if [[ ! -d "${dir}/.git" ]]; then
    # Full clone, not shallow: sessions fetch arbitrary heads into these.
    git clone "${url}" "${dir}"
  fi
  git -C "${dir}" fetch origin
  # Staging and `water package` leave generated state in the checkout: the
  # staged backends/ and examples/ trees (untracked) and a Cargo.lock entry
  # for each staged example (the examples are workspace members, so cargo
  # rewrites the lock). Restore the lockfile when the repo tracks one and
  # check tracked edits only — anything left is real uncommitted work and
  # fails loudly rather than being discarded.
  if git -C "${dir}" ls-files --error-unmatch Cargo.lock >/dev/null 2>&1; then
    git -C "${dir}" checkout -- Cargo.lock
  fi
  if [[ -n "$(git -C "${dir}" status --porcelain --untracked-files=no)" ]]; then
    echo "error: ${dir} has uncommitted changes; refusing to touch it" >&2
    return 1
  fi
  if ! git -C "${dir}" merge-base --is-ancestor origin/dev HEAD; then
    git -C "${dir}" checkout --detach origin/dev
  fi
}

main "$@"
