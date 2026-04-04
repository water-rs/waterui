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
- Continuously update this repo's `.claude/skills/waterui/SKILL.md` whenever WaterUI semantics, testing rules, or major component behavior become clearer during the task. The repo-local skill is part of the product.
- "Visual test" in this repository means the agent reads the generated image directly with its own vision capability. Heuristic image checks are forbidden, including changed-pixel counts, opaque-pixel thresholds, bbox approximations, dominant-color checks, brightness checks, non-uniform checks, and similar proxy code.
- `waterui-testing` is based on the Hydrolysis accessibility tree, not native platform accessibility. Prefer `waterui-testing` for UI component coverage, and treat it as both an interaction test and an accessibility-correctness test.
- Every UI component is expected to produce a meaningful accessibility tree. If a component cannot be covered by `waterui-testing`, treat that as a bug to fix rather than a gap to paper over.
- WaterUI uses fine-grained reactivity with reconstruction semantics. If a view or component is rebuilt by parent control flow, its local state resetting is expected. Do not preserve component-local state across rebuilds unless that state is explicitly owned at the correct reactive level.
- `GpuSurface::new(renderer)` owns one `GpuView` instance for that surface lifetime. `GpuView::setup()` is where persistent GPU resources for that renderer instance belong. Do not move renderer state into hidden shared caches just to survive `GpuSurface` teardown or parent rebuild.
- For text APIs, use `text()` for static text and `text!` for reactive formatting. Do not use `watch()` to build reactive text when `text!` or signal-taking APIs already express the dependency directly.
- Do not write `waterui::text!`. Always import the macro first, then use the short `text!` form.
