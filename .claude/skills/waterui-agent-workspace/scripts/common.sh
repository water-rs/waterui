#!/bin/zsh

# Per-machine locations. Each is overridable through the matching WATERUI_AGENT_*
# environment variable when a machine's layout differs; the defaults are derived
# so nothing user- or agent-specific is frozen into the repository.
typeset -gr WORKSPACE_ROOT="${WATERUI_AGENT_WORKSPACE_ROOT:-${HOME}/.waterui-agent/workspaces}"
typeset -gr LOCK_ROOT="${WATERUI_AGENT_LOCK_ROOT:-${HOME}/.waterui-agent/locks}"
typeset -gr BRANCH_PREFIX="${WATERUI_AGENT_BRANCH_PREFIX:-agent}"

# The canonical source repository, derived from git context rather than hardcoded:
# - run from the canonical checkout (e.g. create_workspace.sh), it is this
#   repository's top level;
# - run from inside an agent workspace clone (sync/finish), the canonical source
#   is that clone's `origin`, which create_workspace.sh points back here.
# Outside any git repository it resolves empty, leaving the per-script context
# checks to report the problem clearly.
#
# Which of the two we are in is decided by what the checkout *is*, not by where
# it sits. A workspace was cloned from a checkout on this machine, so its origin
# is a local path to another working tree; the canonical checkout's origin is
# the remote it is published to. Asking WORKSPACE_ROOT instead — as this did —
# means the answer changes when WATERUI_AGENT_WORKSPACE_ROOT changes, and every
# workspace created under the old value is then read as the canonical
# repository, so sync and finish refuse to run and the work inside them is
# stranded with no way to merge it back.
_waterui_resolve_source_repo() {
  local top origin
  if ! top="$(git rev-parse --show-toplevel 2>/dev/null)"; then
    return 0
  fi
  origin="$(git -C "$top" remote get-url origin 2>/dev/null)" || origin=""
  if [[ -n "$origin" && -d "$origin" ]] &&
    git -C "$origin" rev-parse --show-toplevel >/dev/null 2>&1; then
    print -- "$origin"
  else
    print -- "$top"
  fi
}
typeset -gr SOURCE_REPO="${WATERUI_AGENT_SOURCE_REPO:-$(_waterui_resolve_source_repo)}"

warn() {
  print -u2 -- "warning: $*"
}

die() {
  print -u2 -- "error: $*"
  exit 1
}

# Runs a command, saying nothing unless it fails — and saying everything if it
# does.
#
# Discarding both streams outright is what these calls used to do, and it makes
# a failure undiagnosable: `git clone` reporting "Input/output error" from a
# dying disk surfaced only as "failed to clone canonical repository", which sent
# the reader looking for a git problem that was not there. Letting stderr
# through instead is no good either, because git narrates success on stderr too
# ("Switched to a new branch ..."). So: hold the output, drop it when the
# command succeeds, print it when it does not.
run_quietly() {
  local output

  if ! output="$("$@" 2>&1)"; then
    [[ -z "$output" ]] || print -u2 -- "$output"
    return 1
  fi
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

ensure_nightly_cargo() {
  cargo -V | rg -q 'nightly' || die "nightly cargo is required to inspect Cargo configuration"
}

canonical_dir() {
  local path="$1"
  [[ -d "$path" ]] || die "directory does not exist: $path"
  (
    cd "$path"
    pwd -P
  )
}

device_id() {
  stat -f '%d' "$1"
}

mount_device() {
  df "$1" | awk 'NR == 2 { print $1 }'
}

ensure_apfs() {
  local target="$1"
  local device
  local info

  device="$(mount_device "$target")"
  info="$(diskutil info "$device" 2>/dev/null)" || die "failed to inspect filesystem for $target"
  print -- "$info" | rg -q '^ *Type \(Bundle\): +apfs$' || die "filesystem for $target is not APFS"
}

# `git clone --local` hardlinks the source object store into the clone, which
# cannot span volumes. A cross-volume workspace root copies the objects instead;
# same-volume roots keep the cheap hardlinks. `reference` must be a path that
# already exists on the destination volume.
local_clone_options() {
  local source="$1"
  local reference="$2"

  if [[ "$(device_id "$source")" == "$(device_id "$reference")" ]]; then
    print -- "--local"
  else
    print -- "--local --no-hardlinks"
  fi
}

clone_superproject_locally() {
  local source_root="$1"
  local destination="$2"
  local source_branch="$3"
  local clone_options

  clone_options="$(local_clone_options "$source_root" "${destination:h}")"
  run_quietly git clone ${=clone_options} --branch "$source_branch" --single-branch "$source_root" "$destination" || die "failed to clone canonical repository into $destination"
}

submodule_gitlink_commit() {
  local repo_root="$1"
  local submodule_relpath="$2"
  local commit

  commit="$(git -C "$repo_root" ls-tree HEAD "$submodule_relpath" | awk '{ print $3 }')"
  [[ -n "$commit" ]] || die "failed to resolve gitlink commit for ${submodule_relpath} in $repo_root"
  print -- "$commit"
}

clone_submodules_locally() {
  local source_root="$1"
  local destination_root="$2"
  local name
  local submodule_relpath
  local source_submodule
  local destination_submodule
  local desired_commit
  local clone_options

  clone_options="$(local_clone_options "$source_root" "$destination_root")"
  while IFS=$'\t' read -r name submodule_relpath; do
    [[ -n "${name:-}" ]] || continue
    source_submodule="${source_root}/${submodule_relpath}"
    destination_submodule="${destination_root}/${submodule_relpath}"
    desired_commit="$(submodule_gitlink_commit "$destination_root" "$submodule_relpath")"

    run_quietly git -C "$destination_root" submodule init -- "$submodule_relpath" || die "failed to initialize submodule config for ${submodule_relpath}"
    run_quietly git clone ${=clone_options} --no-checkout "$source_submodule" "$destination_submodule" || die "failed to clone submodule ${submodule_relpath}"
    run_quietly git -C "$destination_submodule" checkout "$desired_commit" || die "failed to checkout ${desired_commit} in submodule ${submodule_relpath}"
    run_quietly git -C "$destination_root" submodule absorbgitdirs -- "$submodule_relpath" || die "failed to absorb gitdir for submodule ${submodule_relpath}"
  done < <(submodule_records "$source_root")
}

# The canonical `target/` is a live cache, and an unrelated build (Xcode, Gradle,
# `water run`) may be writing into it while a workspace is created. rustc's
# per-CGU object files exist only for the duration of one invocation, so a copy
# can enumerate an entry that is gone by the time it is read. That lost race is
# harmless — the entry is a build temporary and cargo rebuilds whatever is
# missing — so it is retried, and the retry no longer enumerates the vanished
# entry. Any other failure, a full destination volume above all, is fatal at
# once rather than retried. Each attempt copies directory *contents* so a retry
# lands on the same path instead of nesting inside the partial copy.
copy_target_entry() {
  local source="$1"
  local destination="$2"
  local attempt
  local errors
  local line

  for attempt in 1 2 3; do
    if [[ -d "$source" && ! -L "$source" ]]; then
      mkdir -p "$destination"
      errors="$(LC_ALL=C cp -cR "$source/." "$destination" 2>&1 >/dev/null)" && return 0
    else
      errors="$(LC_ALL=C cp -cR "$source" "$destination" 2>&1 >/dev/null)" && return 0
    fi

    [[ -n "$errors" ]] || die "failed to copy ${source} into ${destination}"
    for line in ${(f)errors}; do
      [[ "$line" == *': No such file or directory' ]] ||
        die "failed to copy ${source} into ${destination}: ${line}"
    done
  done

  warn "left ${source:t} out of the warm target cache: it kept vanishing under the copy, so another build is writing to ${source:h}"
}

copy_target_cow() {
  local source_root="$1"
  local destination_root="$2"
  local source_target="${source_root}/target"
  local destination_target="${destination_root}/target"
  local top
  local child

  [[ -d "$source_target" ]] || return 0
  mkdir -p "$destination_target"
  # Copy entry-by-entry instead of one `cp -cR target` so the per-profile
  # `incremental/` caches can be skipped: they are the largest and
  # fastest-diverging piece of `target/` (tens of GB), every workspace build
  # rewrites them immediately (turning shared COW blocks into real copies),
  # and sccache already covers the cold recompiles they would have saved.
  # With N concurrent workspaces this is the difference between sharing one
  # dependency cache and carrying N diverging full copies.
  for top in "$source_target"/*(DN); do
    if [[ -d "$top" && ! -L "$top" ]]; then
      mkdir -p "${destination_target}/${top:t}"
      for child in "$top"/*(DN); do
        [[ "${child:t}" == "incremental" ]] && continue
        copy_target_entry "$child" "${destination_target}/${top:t}/${child:t}"
      done
    else
      copy_target_entry "$top" "${destination_target}/${top:t}"
    fi
  done
}

submodule_records() {
  local repo_root="$1"
  git -C "$repo_root" config --file .gitmodules --get-regexp '^submodule\..*\.path$' |
    awk '{ sub(/^submodule\./, "", $1); sub(/\.path$/, "", $1); print $1 "\t" $2 }'
}

configured_submodule_branch() {
  local repo_root="$1"
  local submodule_name="$2"
  git -C "$repo_root" config --file .gitmodules --get "submodule.${submodule_name}.branch" 2>/dev/null || true
}

ensure_source_submodules_ready() {
  local repo_root="$1"
  local name
  local submodule_relpath
  local submodule_path
  local submodule_git_dir

  while IFS=$'\t' read -r name submodule_relpath; do
    [[ -n "${name:-}" ]] || continue
    submodule_path="${repo_root}/${submodule_relpath}"
    [[ -d "$submodule_path" ]] || die "submodule working tree missing: $submodule_relpath"
    submodule_git_dir="$(git -C "$submodule_path" rev-parse --git-dir 2>/dev/null)" || die "submodule is not a valid git worktree: $submodule_relpath"
    [[ "$submodule_git_dir" == "${repo_root}/.git/modules/"* ]] || die "submodule gitdir does not live under source repo metadata: $submodule_relpath -> $submodule_git_dir"
  done < <(submodule_records "$repo_root")
}

ensure_repo_and_submodules_clean() {
  local repo_root="$1"
  local label="$2"
  local name
  local submodule_relpath

  ensure_clean_worktree "$repo_root" "$label"

  while IFS=$'\t' read -r name submodule_relpath; do
    [[ -n "${name:-}" ]] || continue
    ensure_clean_worktree "${repo_root}/${submodule_relpath}" "${label} submodule ${submodule_relpath}"
  done < <(submodule_records "$SOURCE_REPO")
}

# The same question as a yes-or-no, for callers that want to mention the state
# rather than refuse to run.
#
# Asking it through the checking form keeps one definition of "clean": `die`
# ends the subshell rather than this script, so the exit status answers.
repo_and_submodules_are_clean() {
  (ensure_repo_and_submodules_clean "$1" "canonical repository") >/dev/null 2>&1
}

activate_all_submodules() {
  local repo_root="$1"
  git -C "$repo_root" config submodule.active . || die "failed to activate submodules in workspace"
}

ensure_branch() {
  local repo_root="$1"
  local branch_name="$2"

  if git -C "$repo_root" show-ref --verify --quiet "refs/heads/$branch_name"; then
    run_quietly git -C "$repo_root" switch "$branch_name" || die "failed to switch to existing branch $branch_name in $repo_root"
  else
    run_quietly git -C "$repo_root" switch -c "$branch_name" || die "failed to create branch $branch_name in $repo_root"
  fi
}

branch_submodules() {
  local repo_root="$1"
  local branch_name="$2"
  local name
  local submodule_relpath
  local submodule_path

  while IFS=$'\t' read -r name submodule_relpath; do
    [[ -n "${name:-}" ]] || continue
    submodule_path="${repo_root}/${submodule_relpath}"
    git -C "$submodule_path" rev-parse --git-dir >/dev/null 2>&1 || die "submodule is not a valid git worktree after clone: $submodule_relpath"
    ensure_branch "$submodule_path" "$branch_name"
  done < <(submodule_records "$repo_root")
}

ensure_no_shared_target_dir() {
  local configured_target

  if configured_target="$(cargo -Z unstable-options config get build.target-dir 2>/dev/null)"; then
    die "cargo build.target-dir is configured (${configured_target//$'\n'/ }); remove it before creating agent workspaces"
  fi
}

validate_slug() {
  local slug="$1"
  [[ "$slug" =~ '^[a-z0-9]+([a-z0-9-]*[a-z0-9]+)?$|^[a-z0-9]+$' ]] || die "task slug must be lowercase letters, digits, and hyphens"
}

current_branch() {
  local repo_root="$1"
  local branch_name

  branch_name="$(git -C "$repo_root" rev-parse --abbrev-ref HEAD 2>/dev/null)" || die "failed to determine current branch for $repo_root"
  [[ "$branch_name" != "HEAD" ]] || die "repository is in detached HEAD state: $repo_root"
  print -- "$branch_name"
}

ensure_clean_worktree() {
  local repo_root="$1"
  local label="$2"
  local untracked_output
  local filtered_untracked

  git -C "$repo_root" diff --quiet --exit-code || die "${label} has unstaged changes"
  git -C "$repo_root" diff --cached --quiet --exit-code || die "${label} has staged but uncommitted changes"

  untracked_output="$(git -C "$repo_root" ls-files --others --exclude-standard)"
  filtered_untracked="$(print -- "$untracked_output" | rg -v '(^|/)\.worktrees(/|$)|(^|/)\.water(/|$)' || true)"
  [[ -z "$filtered_untracked" ]] || die "${label} has untracked files"
}

ensure_branch_available() {
  local repo_root="$1"
  local branch_name="$2"

  if git -C "$repo_root" show-ref --verify --quiet "refs/heads/$branch_name"; then
    run_quietly git -C "$repo_root" switch "$branch_name" || die "failed to switch to ${branch_name} in $repo_root"
    return
  fi

  if git -C "$repo_root" show-ref --verify --quiet "refs/remotes/origin/$branch_name"; then
    run_quietly git -C "$repo_root" switch -c "$branch_name" --track "origin/$branch_name" || die "failed to create local branch ${branch_name} in $repo_root"
    return
  fi

  die "branch ${branch_name} does not exist in $repo_root"
}

resolve_branch_tip() {
  local repo_root="$1"
  local branch_name="$2"
  local remote_commit
  local local_commit

  if git -C "$repo_root" show-ref --verify --quiet "refs/heads/$branch_name"; then
    local_commit="$(git -C "$repo_root" rev-parse "refs/heads/$branch_name")" || die "failed to resolve branch ${branch_name} in $repo_root"
    print -- "$local_commit"
    return 0
  fi

  if git -C "$repo_root" show-ref --verify --quiet "refs/remotes/origin/$branch_name"; then
    remote_commit="$(git -C "$repo_root" rev-parse "refs/remotes/origin/$branch_name")" || die "failed to resolve origin/${branch_name} in $repo_root"
    print -- "$remote_commit"
    return 0
  fi

  die "branch ${branch_name} does not exist in $repo_root"
}

merge_branch_ff_only() {
  local destination_repo="$1"
  local source_repo="$2"
  local source_branch="$3"
  local label="$4"

  git -C "$source_repo" show-ref --verify --quiet "refs/heads/$source_branch" || die "${label} source branch ${source_branch} does not exist in $source_repo"
  run_quietly git -C "$destination_repo" fetch "$source_repo" "$source_branch" || die "failed to fetch ${label} branch ${source_branch} from $source_repo"
  run_quietly git -C "$destination_repo" merge --ff-only FETCH_HEAD || die "failed to fast-forward merge ${label} from ${source_repo}; update or rebase the workspace first"
}

ensure_fast_forward_possible() {
  local destination_repo="$1"
  local source_repo="$2"
  local source_branch="$3"
  local label="$4"
  local destination_head
  local source_head

  destination_head="$(git -C "$destination_repo" rev-parse HEAD)" || die "failed to resolve ${label} destination HEAD"
  source_head="$(git -C "$source_repo" rev-parse "refs/heads/${source_branch}")" || die "failed to resolve ${label} source branch ${source_branch}"
  git -C "$source_repo" merge-base --is-ancestor "$destination_head" "$source_head" >/dev/null 2>&1 || die "${label} cannot be fast-forwarded from ${source_repo}; update or rebase the workspace first"
}

workspace_contains_source_layout() {
  local workspace_root="$1"
  local name
  local submodule_relpath

  [[ -d "${workspace_root}/.git" ]] || die "workspace is missing .git metadata: $workspace_root"

  while IFS=$'\t' read -r name submodule_relpath; do
    [[ -n "${name:-}" ]] || continue
    [[ -d "${workspace_root}/${submodule_relpath}" ]] || die "workspace is missing submodule path ${submodule_relpath}"
  done < <(submodule_records "$SOURCE_REPO")
}

ensure_workspace_context() {
  local current_root
  local expected_source
  local current_branch

  current_root="$(git rev-parse --show-toplevel 2>/dev/null)" || die "run this script from an agent workspace"
  current_root="$(canonical_dir "$current_root")"
  expected_source="$(canonical_dir "$SOURCE_REPO")"

  [[ "$current_root" != "$expected_source" ]] || die "run this script from an agent workspace, not the canonical repository"

  # Being a clone of a local checkout is not on its own enough to earn the
  # things finish_workspace does — fast-forwarding the canonical repository from
  # here, then deleting this directory. Someone's own second clone would qualify
  # on origin alone. The branch create_workspace.sh puts a workspace on is the
  # other half of the evidence, and unlike a location it travels with the
  # checkout.
  current_branch="$(git -C "$current_root" rev-parse --abbrev-ref HEAD 2>/dev/null)" ||
    die "workspace has no checked-out branch"
  [[ "$current_branch" == "${BRANCH_PREFIX}/"* ]] ||
    die "not an agent workspace: expected a ${BRANCH_PREFIX}/ branch, found ${current_branch}"

  print -- "$current_root"
}

ensure_workspace_branch_consistency() {
  local workspace_root="$1"
  local workspace_branch="$2"
  local name
  local submodule_relpath
  local submodule_path
  local submodule_branch

  while IFS=$'\t' read -r name submodule_relpath; do
    [[ -n "${name:-}" ]] || continue
    submodule_path="${workspace_root}/${submodule_relpath}"
    submodule_branch="$(current_branch "$submodule_path")"
    [[ "$submodule_branch" == "$workspace_branch" ]] || die "submodule ${submodule_relpath} is on ${submodule_branch}, expected ${workspace_branch}"
  done < <(submodule_records "$SOURCE_REPO")
}

integration_lock_dir_for_source() {
  local source_root="$1"
  local lock_root
  local lock_key

  mkdir -p "$LOCK_ROOT" || die "failed to create lock root: $LOCK_ROOT"
  lock_root="$(canonical_dir "$LOCK_ROOT")"
  lock_key="$(print -n -- "$source_root" | shasum -a 256 | awk '{ print $1 }')"
  print -- "${lock_root}/${lock_key}.lock"
}

ensure_no_integration_lock() {
  local source_root="$1"
  local lock_dir

  lock_dir="$(integration_lock_dir_for_source "$source_root")"
  [[ ! -d "$lock_dir" ]] || die "another agent is already integrating into $source_root"
}

acquire_integration_lock() {
  local source_root="$1"
  local lock_dir

  lock_dir="$(integration_lock_dir_for_source "$source_root")"
  mkdir "$lock_dir" 2>/dev/null || die "another agent is already integrating into $source_root"
  print -- "$$" > "${lock_dir}/pid"
  print -- "$WORKSPACE_ROOT" > "${lock_dir}/workspace-root"
  print -- "$source_root" > "${lock_dir}/source-repo"
  print -- "$lock_dir"
}

release_integration_lock() {
  local lock_dir="${1:-}"
  [[ -n "$lock_dir" && -d "$lock_dir" ]] || return 0
  rm -rf "$lock_dir"
}

rebase_current_branch_onto_source_branch() {
  local workspace_repo="$1"
  local source_repo="$2"
  local source_branch="$3"
  local label="$4"

  run_quietly git -C "$workspace_repo" fetch "$source_repo" "$source_branch" || die "failed to fetch ${label} branch ${source_branch} from $source_repo"
  run_quietly git -C "$workspace_repo" rebase FETCH_HEAD || die "rebase stopped in ${label}; resolve conflicts inside the workspace and continue or abort manually"
}
