#!/bin/zsh

set -euo pipefail

SCRIPT_DIR="${0:A:h}"
source "${SCRIPT_DIR}/common.sh"

is_configured_submodule_path() {
  local candidate="$1"
  local name
  local submodule_relpath

  while IFS=$'\t' read -r name submodule_relpath; do
    [[ -n "${name:-}" ]] || continue
    [[ "$candidate" == "$submodule_relpath" ]] && return 0
  done < <(submodule_records "$SOURCE_REPO")

  return 1
}

ensure_workspace_sync_ready() {
  local workspace_root="$1"
  local name
  local submodule_relpath
  local status_output
  local status_line
  local changed_path

  while IFS=$'\t' read -r name submodule_relpath; do
    [[ -n "${name:-}" ]] || continue
    ensure_clean_worktree "${workspace_root}/${submodule_relpath}" "workspace submodule ${submodule_relpath}"
  done < <(submodule_records "$SOURCE_REPO")

  status_output="$(git -C "$workspace_root" status --porcelain=v1 --untracked-files=normal)"
  [[ -n "$status_output" ]] || return 0

  while IFS= read -r status_line; do
    [[ -n "$status_line" ]] || continue
    changed_path="${status_line#?? }"
    is_configured_submodule_path "$changed_path" && continue
    die "workspace has changes outside submodule pointer updates"
  done <<< "$status_output"
}

commit_submodule_pointer_updates_if_needed() {
  local workspace_root="$1"
  local commit_message="$2"
  local name
  local submodule_relpath
  local workspace_submodule
  local recorded_commit
  local actual_commit
  local changed=0

  while IFS=$'\t' read -r name submodule_relpath; do
    [[ -n "${name:-}" ]] || continue
    workspace_submodule="${workspace_root}/${submodule_relpath}"
    recorded_commit="$(submodule_gitlink_commit "$workspace_root" "$submodule_relpath")"
    actual_commit="$(git -C "$workspace_submodule" rev-parse HEAD)" || die "failed to resolve workspace submodule HEAD for ${submodule_relpath}"

    if [[ "$recorded_commit" == "$actual_commit" ]]; then
      continue
    fi

    git -C "$workspace_root" add "$submodule_relpath" || die "failed to stage submodule pointer update for ${submodule_relpath}"
    changed=1
  done < <(submodule_records "$SOURCE_REPO")

  if [[ "$changed" -eq 0 ]]; then
    return 0
  fi

  run_quietly git -C "$workspace_root" commit -m "$commit_message" || die "failed to commit rebased submodule pointers in workspace"
}

sync_submodules() {
  local source_root="$1"
  local workspace_root="$2"
  local name
  local submodule_relpath
  local canonical_submodule
  local workspace_submodule
  local target_branch

  while IFS=$'\t' read -r name submodule_relpath; do
    [[ -n "${name:-}" ]] || continue
    canonical_submodule="${source_root}/${submodule_relpath}"
    workspace_submodule="${workspace_root}/${submodule_relpath}"
    target_branch="$(configured_submodule_branch "$source_root" "$name")"
    [[ -n "$target_branch" ]] || die "submodule ${submodule_relpath} has no configured integration branch in .gitmodules"
    resolve_branch_tip "$canonical_submodule" "$target_branch" >/dev/null
    rebase_current_branch_onto_source_branch "$workspace_submodule" "$canonical_submodule" "$target_branch" "submodule ${submodule_relpath}"
  done < <(submodule_records "$SOURCE_REPO")
}

sync_superproject() {
  local source_root="$1"
  local workspace_root="$2"
  local source_branch

  source_branch="$(current_branch "$source_root")"
  rebase_current_branch_onto_source_branch "$workspace_root" "$source_root" "$source_branch" "superproject"
}

main() {
  local source_root
  local workspace_root
  local workspace_branch

  require_command awk
  require_command git
  require_command rg
  require_command shasum

  source_root="$(canonical_dir "$SOURCE_REPO")"
  workspace_root="$(ensure_workspace_context)"
  workspace_contains_source_layout "$workspace_root"
  ensure_source_submodules_ready "$source_root"
  activate_all_submodules "$source_root"
  activate_all_submodules "$workspace_root"
  ensure_no_integration_lock "$source_root"

  workspace_branch="$(current_branch "$workspace_root")"
  [[ "$workspace_branch" == "${BRANCH_PREFIX}/"* ]] || die "workspace branch must start with ${BRANCH_PREFIX}/"
  ensure_workspace_branch_consistency "$workspace_root" "$workspace_branch"

  ensure_workspace_sync_ready "$workspace_root"
  ensure_repo_and_submodules_clean "$source_root" "canonical repository"

  sync_submodules "$source_root" "$workspace_root"
  commit_submodule_pointer_updates_if_needed "$workspace_root" "Sync rebased submodule pointers"
  ensure_repo_and_submodules_clean "$workspace_root" "workspace after submodule sync"

  sync_superproject "$source_root" "$workspace_root"
  commit_submodule_pointer_updates_if_needed "$workspace_root" "Refresh submodule pointers after syncing canonical repository"
  ensure_repo_and_submodules_clean "$workspace_root" "workspace after sync"
}

[[ $# -eq 0 ]] || die "usage: sync_workspace.sh"
main
