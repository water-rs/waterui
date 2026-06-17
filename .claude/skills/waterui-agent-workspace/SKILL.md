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
- The canonical repository or any canonical submodule has uncommitted tracked changes, staged changes, or untracked non-ignored files.
- Cargo resolves any `build.target-dir`.
- The workspace root is not on the same filesystem as the source repository.
- The source or workspace filesystem is not APFS.
- Any configured submodule is missing, is not a Git worktree, or does not keep its gitdir under the source repository metadata.
- The destination path already exists.
- The task slug is not lowercase hyphenated text.

The finish script fails immediately if any of the following are true:

- It is not run from an agent workspace under `WATERUI_AGENT_WORKSPACE_ROOT`.
- Another agent is already merging back into the canonical repository.
- The workspace superproject or any workspace submodule has uncommitted changes.
- The canonical repository superproject or any canonical submodule has uncommitted changes.
- The workspace branch does not start with `agent/`.
- Any workspace submodule is on a different branch from the workspace superproject.
- Any submodule has no configured integration branch in `.gitmodules`.
- A fast-forward merge is not possible.

For WaterUI's backend submodules, "configured integration branch" means `dev`. If a backend agent branch cannot fast-forward onto `dev`, the agent must rebase and resolve conflicts inside the workspace before running finish again.

The sync script fails immediately if any of the following are true:

- It is not run from an agent workspace under `WATERUI_AGENT_WORKSPACE_ROOT`.
- Another agent is currently running `finish_workspace.sh` against the canonical repository.
- The workspace superproject or any workspace submodule has uncommitted changes.
- The canonical repository superproject or any canonical submodule has uncommitted changes.
- The workspace branch does not start with `agent/`.
- Any workspace submodule is on a different branch from the workspace superproject.
- A submodule rebase or superproject rebase stops for conflicts.

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
