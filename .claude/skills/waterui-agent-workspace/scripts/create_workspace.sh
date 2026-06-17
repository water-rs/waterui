#!/bin/zsh

set -euo pipefail

SCRIPT_DIR="${0:A:h}"
source "${SCRIPT_DIR}/common.sh"

ensure_canonical_repo_context() {
  local current_root
  local expected_root

  current_root="$(git rev-parse --show-toplevel 2>/dev/null)" || die "run this script from the canonical WaterUI repository"
  current_root="$(canonical_dir "$current_root")"
  expected_root="$(canonical_dir "$SOURCE_REPO")"
  [[ "$current_root" == "$expected_root" ]] || die "run this script from $SOURCE_REPO"
}

main() {
  local slug="$1"
  local source_root
  local source_branch
  local workspace_root
  local timestamp
  local destination
  local branch_name
  local source_device
  local workspace_device

  require_command cargo
  require_command cp
  require_command df
  require_command diskutil
  require_command git
  require_command rg
  require_command awk

  ensure_nightly_cargo
  validate_slug "$slug"
  ensure_canonical_repo_context
  ensure_no_shared_target_dir

  source_root="$(canonical_dir "$SOURCE_REPO")"
  source_branch="$(current_branch "$source_root")"
  ensure_source_submodules_ready "$source_root"
  ensure_repo_and_submodules_clean "$source_root" "canonical repository"
  mkdir -p "$WORKSPACE_ROOT"
  workspace_root="$(canonical_dir "$WORKSPACE_ROOT")"

  source_device="$(device_id "$source_root")"
  workspace_device="$(device_id "$workspace_root")"
  [[ "$source_device" == "$workspace_device" ]] || die "workspace root must be on the same filesystem as $SOURCE_REPO"

  ensure_apfs "$source_root"
  ensure_apfs "$workspace_root"

  timestamp="$(date '+%Y%m%d-%H%M%S')"
  destination="${workspace_root}/${timestamp}-${slug}"
  branch_name="${BRANCH_PREFIX}/${slug}/${timestamp}"

  [[ ! -e "$destination" ]] || die "destination already exists: $destination"

  print -u2 -- "creating local git clone at $destination"
  clone_superproject_locally "$source_root" "$destination" "$source_branch"

  clone_submodules_locally "$source_root" "$destination"
  activate_all_submodules "$destination"
  copy_target_cow "$source_root" "$destination"
  ensure_branch "$destination" "$branch_name"
  branch_submodules "$destination" "$branch_name"

  print -- "$destination"
}

[[ $# -eq 1 ]] || die "usage: create_workspace.sh <task-slug>"
main "$1"
