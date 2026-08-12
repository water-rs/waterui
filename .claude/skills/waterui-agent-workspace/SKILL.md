---
name: waterui-agent-workspace
description: Manage isolated development workspaces for substantial code changes in the WaterUI repository. Use whenever an agent is asked to implement a feature, build a new capability, perform a large bug fix, refactor multiple files, modify backends or submodules, run a parallel agent task, or do any non-trivial WaterUI development that will involve writing code, running repeated builds or tests, rebasing, resolving conflicts, or merging back. For these tasks, do not edit the canonical repository directly. This skill creates a local git-cloned workspace with a COW-copied Cargo target, clones WaterUI submodules safely, explains how to resolve conflicts inside the workspace, and finishes by taking the global integration lock, fast-forwarding changes back, and deleting the workspace.
---

# WaterUI Agent Workspace

Use this skill before making substantial changes to WaterUI. Treat the WaterUI checkout you are working in as the canonical source repository. For substantial work, do not edit it directly; create a private workspace first.

## Run The Workflow

1. Decide whether the task requires isolation.
Use an isolated workspace for new features, large refactors, backend changes, submodule work, parallel agent tasks, or any change that will need multiple build/test cycles.
Do not use it for read-only investigation or tiny localized edits that do not justify a separate workspace.

2. Ensure the canonical repository and every initialized submodule are committed and clean.
This workflow now uses `git clone`, so uncommitted tracked changes in the canonical repository are a hard stop.

3. From the canonical WaterUI checkout's repository root, run:

```bash
.claude/skills/waterui-agent-workspace/scripts/create_workspace.sh <task-slug>
```

Use a short lowercase hyphenated slug such as `layout-cache-fix`.

4. Change into the printed workspace path and do all subsequent edits, builds, and tests there.

Verification ownership inside the workspace:

- Once you choose this isolated workspace, you own the state of that workspace for the duration of the task.
- Do not describe a verification blocker as "not introduced by this turn", "pre-existing in this workspace", or similar blame-shifting language. In this workflow, that still means you have not yet brought the workspace to a verifiable state.
- If a requested test, build, or review is blocked by a compile error, failing dependency edge, broken local file, or other issue inside the current editable WaterUI workspace, fix that blocker first when it is part of the editable codebase and required to complete the user's requested verification.
- Do not stop at reporting the blocker unless the required next change would violate an explicit repository rule or requires user product direction. Otherwise, continue until the originally requested verification actually runs.
- When you must report incomplete verification, say plainly that you have not finished because the workspace is not yet back to a verifiable state, then continue repairing it instead of framing the failure as someone else's change.

WaterUI submodule boundary:

- `backends/apple`, `backends/android`, `kit`, and `utils/nami` are first-party WaterUI repositories managed through this workspace flow, not third-party upstream crates.
- When the task requires changes inside those submodules, make those changes in the agent workspace and commit them on the workspace submodule branch.
- Do not treat compile failures inside those submodules as "upstream crate" issues that must be deferred by default. They are part of the editable WaterUI codebase.

5. While the workspace is active, keep rebasing it onto the canonical repository when needed.
Resolve conflicts inside the workspace, never in the canonical repository.
For WaterUI, rebase changed submodules first, then update the superproject submodule pointers, then rebase the superproject branch.
For `backends/apple` and `backends/android`, the integration target is the `dev` branch declared in `.gitmodules`. Keep backend conflicts and rebases inside the workspace until those backend agent branches are ready to fast-forward `dev`.
Use this helper when the workspace falls behind:

```bash
.claude/skills/waterui-agent-workspace/scripts/sync_workspace.sh
```

Run it from inside the workspace. It rebases submodules first, records any rebased submodule pointer updates in the workspace superproject, then rebases the superproject itself.
If the canonical superproject and the workspace both changed the same submodule pointer, the superproject rebase may still stop for conflicts. Resolve that conflict inside the workspace, continue or abort the rebase there, and only return to `finish_workspace.sh` after the workspace is clean again.

6. When development is complete, commit all intended changes in the workspace superproject and in every changed submodule. Then run:

```bash
.claude/skills/waterui-agent-workspace/scripts/finish_workspace.sh
```

Run it from anywhere inside the agent workspace, not from the canonical repository.

7. Never use `git worktree` for this repository.

8. Never set `CARGO_TARGET_DIR` or Cargo `build.target-dir` for this repository. This workflow depends on per-workspace `target/` directories plus shared `sccache`.

Override the default source or workspace paths only through `WATERUI_AGENT_SOURCE_REPO` or `WATERUI_AGENT_WORKSPACE_ROOT` when the machine layout changes.

The created workspace is not only branched at the superproject level. The script clones each configured submodule from the local canonical checkout, activates it in the new workspace, and creates the same `agent/<slug>/<timestamp>` branch inside each initialized submodule so backend work does not happen on detached HEADs.

The workspace is created with `git clone`, so Git naturally skips ignored and untracked build debris such as `.worktrees` and `.water`. After cloning, the script uses APFS COW copy for the repository `target/` directory so Rust builds stay warm without sharing Cargo locks.

The finish step is serialized across all agent workspaces with a lock on the canonical repository. Only one agent may merge back at a time.

## What The Script Enforces

The create script fails immediately if any of the following are true:

- It is not run from the canonical source repository.
- Cargo resolves any `build.target-dir`.
- The source or workspace filesystem is not APFS.

Uncommitted changes in the canonical checkout do not stop it, and are reported
rather than obeyed: a workspace is built from committed state, so whatever is
in someone's working tree has no bearing on what lands in it. Requiring a clean
tree here protected nothing while letting one agent's half-finished edit stop
every other agent from starting.

A workspace root on a different volume than the source repository is allowed
(the usual answer when the boot disk is out of space) but only warns: APFS
clonefile cannot span volumes, so the `target/` copy degrades from
copy-on-write to a full byte copy — creation is slower and the copy occupies
real disk space instead of sharing blocks. The difference is large enough to
change behaviour: seconds against tens of minutes. A root on a removable volume
costs more than time, because a workspace is unusable while that volume is
detached and `finish_workspace.sh` can wedge on it — see below.
- Any configured submodule is missing, is not a Git worktree, or does not keep its gitdir under the source repository metadata.
- The destination path already exists.
- The task slug is not lowercase hyphenated text.

The finish script fails immediately if any of the following are true:

- It is not run from inside an agent workspace. A workspace is recognised by
  what the checkout is — a clone whose `origin` is a local path to another
  checkout, sitting on an `agent/` branch — never by where it sits, so
  moving `WATERUI_AGENT_WORKSPACE_ROOT` does not strand existing workspaces.
- Another agent is already merging back into the canonical repository.
- The workspace superproject or any workspace submodule has uncommitted changes.
- The canonical repository superproject or any canonical submodule has uncommitted changes.
- The workspace branch does not start with `agent/`.
- Any workspace submodule is on a different branch from the workspace superproject.
- Any submodule has no configured integration branch in `.gitmodules`.
- A fast-forward merge is not possible.

For WaterUI's backend submodules, "configured integration branch" means `dev`. If a backend agent branch cannot fast-forward onto `dev`, the agent must rebase and resolve conflicts inside the workspace before running finish again.

The sync script fails immediately if any of the following are true:

- It is not run from inside an agent workspace (same recognition rule as the
  finish script: local-path `origin` plus an `agent/` branch).
- Another agent is currently running `finish_workspace.sh` against the canonical repository.
- The workspace superproject or any workspace submodule has uncommitted changes.
- The canonical repository superproject or any canonical submodule has uncommitted changes.
- The workspace branch does not start with `agent/`.
- Any workspace submodule is on a different branch from the workspace superproject.
- A submodule rebase or superproject rebase stops for conflicts.

## When Merging Back Is Blocked

Finish and sync do require a clean canonical checkout, because they write to it.
That state is routine here and it is almost never yours: several sessions share
the one checkout, and its uncommitted files usually belong to an agent still
working in it.

Treat another session's working tree as untouchable. Do not stash it, revert it,
commit it, or move it aside "just for a moment" — the file you take away is
live, not stale. Stashing to unblock a merge cost a commit: the owning session
ran `git add` during the seconds its file was stashed, and the commit it
produced described a fix while containing only half of it. Restoring the stash
then resurrected a diagnostic edit that session had already tried and discarded,
which it caught only by reading the diff line by line.

Say something instead. The sessions are addressable, and asking is cheaper than
waiting blindly:

1. `mcp__ccd_session_mgmt__list_sessions` — the owner is normally the running
   session whose title matches the dirty paths, near the top of the list.
2. `mcp__ccd_session_mgmt__send_message` — name the exact paths blocking you and
   ask that session to commit or stash them itself. It knows whether its own
   work is in a committable state; you do not.
3. Wait on the condition rather than polling by hand — a background `until`
   loop over `git status --porcelain` wakes you once, when it clears.

If nothing owns the changes — a session that has since exited — ask the user
rather than deciding for them.

## When Finish Does Not Return

`finish_workspace.sh` holds a global integration lock under
`~/.waterui-agent/locks/` from before the merge until it exits, so while it runs
no other agent can merge. It releases the lock from an `EXIT INT TERM` trap.

Never conclude it succeeded because its effects appeared. `delete_workspace` is
the last thing it does, so the commits land on canonical well before the script
is done. One run whose workspace sat on a USB volume that dropped off the bus
left its `rm -rf` in uninterruptible disk wait for eighty minutes; the merge had
landed, canonical was clean, and every other agent's finish failed with "another
agent is already integrating" the whole time.

The lock records its holder, so this is a question you can answer rather than
infer — from either side. The lock directory contains `pid`, `source-repo` and
`workspace-root`:

    ls ~/.waterui-agent/locks/                       # empty means nobody holds it
    ps -p "$(cat ~/.waterui-agent/locks/*.lock/pid)" # alive, or a lock left behind

Run it after your own finish to confirm the script really exited, and run it
when yours is refused to see whether the holder is a live merge or a wedged one.
It is how the eighty-minute stall above was diagnosed — by the blocked session,
not by the one holding the lock.

A lock held by a wedged run cannot be cleared with `kill -TERM`: zsh defers the
trap until the foreground command returns, and a process in uninterruptible wait
never returns. Force-unmounting the volume lets the `rm` fail, after which the
script finishes and cleans up after itself. `kill -9` skips the trap, so the
lock directory must then be removed by hand.

## Moving The Workspace Root

`WATERUI_AGENT_WORKSPACE_ROOT` may change between sessions — typically to an
external volume when the boot disk fills up. What to know when it does:

- Existing workspaces stay where they were created and remain fully usable:
  sync and finish recognise a workspace by its own evidence (local-path
  `origin`, `agent/` branch), not by its location, so no environment override
  is needed for them.
- Exception: a workspace **cloned before that recognition rule landed**
  carries a pre-fix script copy that still path-matches against the current
  root, and running its own `sync_workspace.sh`/`finish_workspace.sh` dies
  with "run this script from an agent workspace, not the canonical
  repository". The scripts decide context from the working directory, not
  from their own path — so run the **canonical checkout's** script by
  absolute path while `cd`'d inside the workspace, and it works without any
  override. (Prefixing `WATERUI_AGENT_WORKSPACE_ROOT=<old root>` onto the
  stale copy also works, but fixes nothing going forward.)
- A root on another volume must still be APFS, and every `create` there pays
  a full `target/` copy instead of a COW clone (see above). If the boot disk
  has room again, prefer a root on the source repository's own volume.

## After Creation

- Run `git status --short` in the new workspace to confirm the starting state.
- Keep all commits, builds, tests, and debugging inside the new workspace.
- Rebase and resolve conflicts only inside the workspace.
- When the canonical repository advances, run `sync_workspace.sh` from the workspace before trying to finish.
- Do not merge back until the workspace and every changed submodule are fully committed and clean.
- Run `finish_workspace.sh` to fast-forward submodules first, then fast-forward the superproject, then delete the workspace directory.
- If finish fails because the canonical repository moved ahead, stop and rebase or respawn the workspace. Do not force a merge inside the canonical repository.

## Resource

- `scripts/create_workspace.sh`: Create the isolated workspace with local `git clone`, clone each submodule from the canonical checkout, copy `target/` with APFS COW, and switch the superproject plus each initialized submodule onto matching `agent/<slug>/<timestamp>` branches.
- `scripts/sync_workspace.sh`: Rebase workspace submodules onto their configured integration branches, commit any rebased submodule pointer updates inside the workspace, then rebase the workspace superproject onto the canonical branch.
- `scripts/finish_workspace.sh`: Acquire the canonical integration lock, fast-forward each configured submodule, fast-forward the superproject, and delete the workspace on success.
- `references/waterui-runtime-semantics.md`: WaterUI-specific guidance for fine-grained rebuild semantics, `GpuSurface` renderer ownership, and the rule that visual tests require direct image reading rather than heuristics.
