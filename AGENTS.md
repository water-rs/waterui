Please fully read CLAUDE.md and follow the instructions before you start to work on the task.

DO NOT be over-engineer or write defensive code. If you encounter a problem, ask user for solution with your own idea, do not say "Let's have a simpler approach". You are expected to face the real problem and make code clean, reusable and elegant. Never take a workaround.

Keep the change set strictly scoped to the task.

- Do not drag unrelated files into the diff.
- Do not run workspace-wide formatters or refactors such as `cargo fmt --all`, bulk codemods, or broad search-replace when the task only targets a few files.
- Prefer file-scoped formatting and verification on the exact files you intentionally changed.
- Do not run multiple `cargo` commands in parallel. It only creates lock contention and provides no benefit in this repository.
- Do not hardcode versions, repository URLs, package sources, filesystem paths, or other environment-derived constants just to ignore real complexity. If a value has a real source of truth, derive it from metadata, build inputs, repository structure, or runtime context instead of freezing a literal.
- Check `git status --short` before and after formatting or codegen steps. If unrelated files appear, stop and narrow the command instead of continuing with a polluted diff.
- Only use repo-wide formatting or sweeping rewrites when the user explicitly asks for them or the task genuinely requires touching the whole workspace.
- Please use `waterui` skill and `waterui-agent-workspace` skill if they exist.
