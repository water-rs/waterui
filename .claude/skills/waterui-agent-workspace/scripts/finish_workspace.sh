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

# Returns a slot to FREE once its work has landed on canonical, instead of
# deleting it — that is the whole point of the slot model (see common.sh):
# the clone and its `target/` stay put, warm, for the next task to reuse.
#
# By this point `merge_submodules_back` and the superproject fast-forward
# already landed the work on canonical, with both canonical submodules and the
# canonical superproject left checked out on their integration branches — so
# each is a ready-made fast-forward source. For each submodule: switch the
# slot off the now-merged agent branch onto its own configured integration
# branch (creating it at the agent branch's tip the first time a slot is
# freed, which is exactly canonical's new tip since the merge just made them
# equal; fast-forwarding an already-existing local branch the next time),
# fast-forward it from canonical to be sure, then delete the spent agent
# branch. Same shape for the superproject, done last.
release_slot_to_free() {
  local workspace_root="$1"
  local source_root="$2"
  local integration_branch="$3"
  local workspace_branch="$4"
  local name
  local submodule_relpath
  local submodule_path
  local canonical_submodule
  local target_branch

  while IFS=$'\t' read -r name submodule_relpath; do
    [[ -n "${name:-}" ]] || continue
    submodule_path="${workspace_root}/${submodule_relpath}"
    canonical_submodule="${source_root}/${submodule_relpath}"
    target_branch="$(configured_submodule_branch "$source_root" "$name")"
    [[ -n "$target_branch" ]] || die "submodule ${submodule_relpath} has no configured integration branch in .gitmodules"

    ensure_branch "$submodule_path" "$target_branch"
    merge_branch_ff_only "$submodule_path" "$canonical_submodule" "$target_branch" "submodule ${submodule_relpath}"
    run_quietly git -C "$submodule_path" branch -D "$workspace_branch" || die "failed to delete spent agent branch ${workspace_branch} in submodule ${submodule_relpath}"
  done < <(submodule_records "$source_root")

  ensure_branch "$workspace_root" "$integration_branch"
  merge_branch_ff_only "$workspace_root" "$source_root" "$integration_branch" "superproject"
  run_quietly git -C "$workspace_root" branch -D "$workspace_branch" || die "failed to delete spent agent branch ${workspace_branch} in superproject"

  print -u2 -- "slot ${workspace_root:t} released back to the pool, warm and FREE"
}

# Removes a legacy timestamped workspace once its work has landed. Slots (see
# common.sh) are returned to FREE instead — this path only survives for
# workspaces created before the slot model, which are recognised by not
# matching the `slot-<N>` basename pattern.
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

  if is_slot_workspace "$workspace_root"; then
    release_slot_to_free "$workspace_root" "$source_root" "$source_branch" "$workspace_branch"
  else
    delete_workspace "$workspace_root"
  fi
}

[[ $# -eq 0 ]] || die "usage: finish_workspace.sh"
main
