#!/bin/zsh

set -euo pipefail

SCRIPT_DIR="${0:A:h}"
source "${SCRIPT_DIR}/common.sh"

typeset -g INTEGRATION_LOCK=""

merge_submodules_back() {
  local source_root="$1"
  local workspace_root="$2"
  local workspace_branch="$3"
  local name
  local submodule_relpath
  local canonical_submodule
  local workspace_submodule
  local target_branch
  local canonical_head
  local workspace_head

  while IFS=$'\t' read -r name submodule_relpath; do
    [[ -n "${name:-}" ]] || continue
    canonical_submodule="${source_root}/${submodule_relpath}"
    workspace_submodule="${workspace_root}/${submodule_relpath}"
    canonical_head="$(git -C "$canonical_submodule" rev-parse HEAD)" || die "failed to resolve canonical submodule HEAD for ${submodule_relpath}"
    workspace_head="$(git -C "$workspace_submodule" rev-parse "refs/heads/${workspace_branch}")" || die "failed to resolve workspace submodule branch ${workspace_branch}"

    if [[ "$canonical_head" == "$workspace_head" ]]; then
      continue
    fi

    target_branch="$(configured_submodule_branch "$source_root" "$name")"
    [[ -n "$target_branch" ]] || die "submodule ${submodule_relpath} has no configured integration branch in .gitmodules"
    ensure_branch_available "$canonical_submodule" "$target_branch"
    merge_branch_ff_only "$canonical_submodule" "$workspace_submodule" "$workspace_branch" "submodule ${submodule_relpath}"
  done < <(submodule_records "$SOURCE_REPO")
}

preflight_fast_forward_merges() {
  local source_root="$1"
  local workspace_root="$2"
  local workspace_branch="$3"
  local name
  local submodule_relpath
  local canonical_submodule
  local workspace_submodule
  local target_branch
  local canonical_head
  local workspace_head
  local target_tip

  while IFS=$'\t' read -r name submodule_relpath; do
    [[ -n "${name:-}" ]] || continue
    canonical_submodule="${source_root}/${submodule_relpath}"
    workspace_submodule="${workspace_root}/${submodule_relpath}"
    canonical_head="$(git -C "$canonical_submodule" rev-parse HEAD)" || die "failed to resolve canonical submodule HEAD for ${submodule_relpath}"
    workspace_head="$(git -C "$workspace_submodule" rev-parse "refs/heads/${workspace_branch}")" || die "failed to resolve workspace submodule branch ${workspace_branch}"

    if [[ "$canonical_head" == "$workspace_head" ]]; then
      continue
    fi

    target_branch="$(configured_submodule_branch "$source_root" "$name")"
    [[ -n "$target_branch" ]] || die "submodule ${submodule_relpath} has no configured integration branch in .gitmodules"
    target_tip="$(resolve_branch_tip "$canonical_submodule" "$target_branch")"
    git -C "$workspace_submodule" merge-base --is-ancestor "$target_tip" "$workspace_head" >/dev/null 2>&1 || die "submodule ${submodule_relpath} cannot be fast-forwarded onto ${target_branch}; update or rebase the workspace first"
  done < <(submodule_records "$SOURCE_REPO")

  ensure_fast_forward_possible "$source_root" "$workspace_root" "$workspace_branch" "superproject"
}

# Removes the workspace once its work has landed.
#
# By this point the merge is already in the canonical repository, so a failure
# here costs a directory, not any work — and it must not read as though the
# integration failed. `rm -rf` gives up outright when the tree grows a file while
# it is being walked, which a build, an editor or the file-system indexer can all
# still be doing seconds after the last command returned, so retry before
# concluding anything is actually stuck.
delete_workspace() {
  local workspace_root="$1"
  local source_root
  local attempt
  local survivors

  source_root="$(canonical_dir "$SOURCE_REPO")"
  cd "$source_root"

  for attempt in 1 2 3 4 5; do
    if rm -rf "$workspace_root" 2>/dev/null && [[ ! -e "$workspace_root" ]]; then
      return 0
    fi
    sleep "$attempt"
  done

  survivors="$(find "$workspace_root" -type f 2>/dev/null | head -5)"
  print -u2 -- ""
  print -u2 -- "integration SUCCEEDED: the workspace branch is merged and the lock is released."
  print -u2 -- "Only the cleanup failed — ${workspace_root} could not be removed after 5 attempts,"
  print -u2 -- "which means something is still writing into it. Nothing is lost; remove it with:"
  print -u2 -- ""
  print -u2 -- "  rm -rf ${workspace_root}"
  if [[ -n "$survivors" ]]; then
    print -u2 -- ""
    print -u2 -- "Files still present (first 5):"
    print -u2 -- "$survivors"
  fi
  return 1
}

main() {
  local workspace_root
  local source_root
  local workspace_branch
  local source_branch
  require_command awk
  require_command cargo
  require_command cp
  require_command df
  require_command diskutil
  require_command find
  require_command git
  require_command rg
  require_command rm
  require_command shasum

  ensure_nightly_cargo
  ensure_no_shared_target_dir

  source_root="$(canonical_dir "$SOURCE_REPO")"
  workspace_root="$(ensure_workspace_context)"
  workspace_contains_source_layout "$workspace_root"
  ensure_source_submodules_ready "$source_root"
  activate_all_submodules "$source_root"
  activate_all_submodules "$workspace_root"

  workspace_branch="$(current_branch "$workspace_root")"
  [[ "$workspace_branch" == "${BRANCH_PREFIX}/"* ]] || die "workspace branch must start with ${BRANCH_PREFIX}/"
  ensure_workspace_branch_consistency "$workspace_root" "$workspace_branch"

  INTEGRATION_LOCK="$(acquire_integration_lock "$source_root")"
  trap 'release_integration_lock "$INTEGRATION_LOCK"' EXIT INT TERM

  ensure_repo_and_submodules_clean "$workspace_root" "workspace"
  ensure_repo_and_submodules_clean "$source_root" "canonical repository"

  source_branch="$(current_branch "$source_root")"
  [[ -n "$source_branch" ]] || die "canonical repository is not on an integration branch"
  preflight_fast_forward_merges "$source_root" "$workspace_root" "$workspace_branch"

  merge_submodules_back "$source_root" "$workspace_root" "$workspace_branch"
  merge_branch_ff_only "$source_root" "$workspace_root" "$workspace_branch" "superproject"
  delete_workspace "$workspace_root"
}

[[ $# -eq 0 ]] || die "usage: finish_workspace.sh"
main
