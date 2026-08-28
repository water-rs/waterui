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

# Clones a brand new slot from canonical the same way this workflow always
# has: local `git clone` of the superproject and every submodule, activated,
# then branched to `branch_name`. Only reached when no existing slot is FREE.
create_new_slot() {
  local source_root="$1"
  local slot="$2"
  local integration_branch="$3"
  local branch_name="$4"

  print -u2 -- "creating local git clone at $slot"
  clone_superproject_locally "$source_root" "$slot" "$integration_branch"
  clone_submodules_locally "$source_root" "$slot"
  activate_all_submodules "$slot"
  # Branch before warming `target/`: the branches are what make the workspace
  # usable, the cache copy is the long step that can still fail, and a workspace
  # left sitting on the canonical branch silently violates the agent-branch
  # workflow.
  ensure_branch "$slot" "$branch_name"
  branch_submodules "$slot" "$branch_name"
}

main() {
  local slug="$1"
  local source_root
  local integration_branch
  local workspace_root
  local timestamp
  local branch_name
  local claim_lock
  local slot
  local is_new_slot
  local source_device
  local workspace_device

  require_command cargo
  require_command cp
  require_command df
  require_command diskutil
  require_command git
  require_command ps
  require_command rg
  require_command awk
  require_command shasum

  ensure_nightly_cargo
  validate_slug "$slug"
  ensure_canonical_repo_context
  ensure_no_shared_target_dir

  source_root="$(canonical_dir "$SOURCE_REPO")"
  integration_branch="$(current_branch "$source_root")"
  ensure_source_submodules_ready "$source_root"

  # A workspace is built from committed state — `git clone` copies refs, and each
  # submodule is checked out at the gitlink recorded in HEAD — so whatever is
  # uncommitted in the canonical checkout has no bearing on what lands here.
  # Refusing to run therefore protected nothing, while meaning one agent's
  # half-finished edit stopped every other agent from *starting*, on a repository
  # several sessions share. Say what will be left behind and carry on.
  # `finish_workspace.sh` still requires a clean canonical: that one writes to it.
  if ! repo_and_submodules_are_clean "$source_root"; then
    warn "canonical repository has uncommitted changes; this workspace starts from committed HEAD and does not include them"
  fi

  mkdir -p "$WORKSPACE_ROOT"
  workspace_root="$(canonical_dir "$WORKSPACE_ROOT")"

  timestamp="$(date '+%Y%m%d-%H%M%S')"
  branch_name="${BRANCH_PREFIX}/${slug}/${timestamp}"

  # See the slot model comment in common.sh for why reuse is the point. The
  # claim lock only needs to cover slot *selection* — scanning for FREE,
  # then switching one to `branch_name` — so it is released the moment that
  # is done, whether the claimed slot was an existing one or a brand new one,
  # and never held across the slow `target/` seed copy below.
  claim_lock="$(acquire_slot_claim_lock "$workspace_root")"
  trap 'release_slot_claim_lock "$claim_lock"' EXIT INT TERM

  is_new_slot=0
  if slot="$(find_and_claim_free_slot "$workspace_root" "$source_root" "$integration_branch" "$branch_name")"; then
    print -u2 -- "reusing warm slot $slot"
  else
    is_new_slot=1
    slot="$(new_slot_path "$workspace_root")"
    print -u2 -- "no reusable slot; creating $slot"

    # APFS is required only where a copy actually happens: seeding a brand new
    # slot's `target/` with `cp -c`. Reusing a slot copies nothing, so a reused
    # slot never pays this check, even on a workspace root that would fail it.
    ensure_apfs "$source_root"
    ensure_apfs "$workspace_root"

    # A slot on the same volume as the source repository clones `target/` with
    # APFS copy-on-write, which is why the default workspace root lives under
    # $HOME. A different volume is still allowed — it is the usual answer when
    # the boot disk is out of space — but clonefile cannot span volumes, so
    # `cp -c` degrades to a full byte copy: slower to create, and the copy
    # occupies real space instead of sharing blocks. This is a one-time cost
    # per slot; every later reuse of this same slot pays nothing.
    source_device="$(device_id "$source_root")"
    workspace_device="$(device_id "$workspace_root")"
    if [[ "$source_device" != "$workspace_device" ]]; then
      warn "workspace root is on a different volume than ${SOURCE_REPO}; target/ will be copied in full rather than cloned"
    fi

    create_new_slot "$source_root" "$slot" "$integration_branch" "$branch_name"
  fi

  release_slot_claim_lock "$claim_lock"
  trap - EXIT INT TERM

  if [[ "$is_new_slot" -eq 1 ]]; then
    copy_target_cow "$source_root" "$slot"
  fi

  print -- "$slot"
}

[[ $# -eq 1 ]] || die "usage: create_workspace.sh <task-slug>"
main "$1"
