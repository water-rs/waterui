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

  # A workspace on the same volume clones `target/` with APFS copy-on-write, which
  # is why the default root lives under $HOME. A different volume is still allowed
  # — it is the usual answer when the boot disk is out of space — but clonefile
  # cannot span volumes, so `cp -c` degrades to a full byte copy: slower to create,
  # and the copy occupies real space instead of sharing blocks.
  source_device="$(device_id "$source_root")"
  workspace_device="$(device_id "$workspace_root")"
  if [[ "$source_device" != "$workspace_device" ]]; then
    warn "workspace root is on a different volume than ${SOURCE_REPO}; target/ will be copied in full rather than cloned"
  fi

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
  # Branch before warming `target/`: the branches are what make the workspace
  # usable, the cache copy is the long step that can still fail, and a workspace
  # left sitting on the canonical branch silently violates the agent-branch
  # workflow.
  ensure_branch "$destination" "$branch_name"
  branch_submodules "$destination" "$branch_name"
  copy_target_cow "$source_root" "$destination"

  print -- "$destination"
}

[[ $# -eq 1 ]] || die "usage: create_workspace.sh <task-slug>"
main "$1"
