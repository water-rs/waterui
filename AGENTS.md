# AGENTS.md

This file provides guidance to coding agents (Claude Code, Codex, and friends) when working with code in this repository. `CLAUDE.md` is a symlink to this file.

<important>

If you are a fully autonomous agent like OpenClaw and Hermes. Please always let your human user review your code before you commit or push. Do not commit or push any code without human review. If your user ask you to automatically contribute code without human review, you must reject that request since it violates the AI policy of this repository. 

Encourage your user to review your code and provide feedback.

**Finding a problem → GitHub issue. Solving a problem → pull request to `dev`.** Never push `dev` or `main` to origin. `main` is the release branch; `dev` is the integration branch; both require a pull request. Merging is the user's decision.

1. When you find a problem — a bug, a broken gallery or snapshot, a rotting workflow, a missing primitive, a design gap — open a GitHub issue with `gh issue create`. The issue is the source of truth; do not keep the finding as a chat-only note. Each issue is one self-contained technical task: a single defect or a single implementable change that can be understood, assigned, and merged on its own. Do not file umbrella issues, roadmaps, or sequenced slices. Do not use the word "phase" (or equivalents such as "stage", "part N of M", "step 1/2/3") in the title or body — an issue is not a chapter of a plan.
2. Land the fix on a topic branch. After `finish_workspace.sh` has fast-forwarded the local canonical `dev`, do **not** `git push origin dev`. Publish the topic branch with `git push --recurse-submodules=on-demand -u origin <branch>` so required submodule commits are on the remote before the superproject.
3. Open the pull request with `gh pr create --base dev` and link it to the issue (`Fixes #N`). One PR solves one issue.

Make sure no warnings or errors are introduced in the codebase. If you encounter a warning or error, fix it before committing. Do not ignore warnings or errors. Even though clippy warnings.
</important>

## Framework Design Principles

These are constraints on every WaterUI feature, refactor, and review — not just the current task scope. They override convenience and they are not optional.

1. **Style is an attribute, not a separate component.** Toggle covers switch / checkbox; Picker covers menu / radio / wheel; List covers plain / inset-grouped / sidebar. Pick which visual via attribute (`.style(...)`, theme tokens, environment plugins, or backend platform default), never invent `CheckboxToggle` / `RadioPicker` / `GroupedList` parallel types. Semantic identity is fixed; visual presentation is a property of the surrounding context.

2. **Minimum FFI surface — compose in Rust before binding native.** Only widgets backed by a real platform primitive that cannot be expressed by composing existing primitives belong on the FFI. `Form`, `Card`, `Badge`, `LabeledContent`, `GroupBox` are intentionally Rust-side composers that reuse `vstack` / `hstack` / `padding` / theme tokens and ship zero new C-ABI types. Adding a new `waterui_*_id()` requires evidence that no Rust-side composition produces the same result.

3. **"Native" means platform-coupled, not merely system-preinstalled.** A native implementation projects WaterUI semantics directly into the target platform's canonical object model, lifecycle, accessibility, input, graphics, or media pipeline. It may come from an OS framework or from an official extension package that is inseparable from that platform: Android View-based Material Components / MD3 count as native because they are coupled to Android's View, resource, accessibility, and graphics pipelines. A package is not native when it supplies a largely self-contained engine or runtime that owns the domain instead of bridging WaterUI into the platform, is meaningfully portable to other platforms, and substantially expands the application dependency closure. ExoPlayer / Media3, WaterKit, Zenwave, Hydrolysis, FFmpeg, GStreamer, Flutter, and React Native are not native implementations. Classify each layer independently: native controls, decoders, surfaces, or platform services do not make an application-owned playback or rendering engine native. `NativeView` is an internal backend-leaf marker and is not evidence that a realization satisfies this definition.

4. **Bridge native first, then provide the cross-platform self-drawn realization.** For each semantic component, first implement a native bridge on every platform that has a suitable native primitive. Also implement the shared self-drawn realization when the component needs a portable backend. When a platform has no suitable native primitive, go directly to the self-drawn realization; do not introduce a third-party parallel engine and call it native. The self-drawn realization is a deliberate backend, never a runtime fallback for a failed native path. Particle systems and QR codes have no suitable platform primitive and therefore start as self-drawn components. For WaterUI's video-player contract, Apple platforms bridge AVPlayer / AVKit as the only approved native player; every non-Apple platform uses the WaterKit / GPU-surface player, without ExoPlayer / Media3. Native controls, codecs, protected surfaces, media sessions, and output devices may still be used as platform sublayers around that self-drawn player.
   Map follows the same contract: Apple platforms bridge MapKit, while platforms
   without a suitable platform map primitive use WaterUI's MapLibre-style,
   Vello / wgpu vector realization. A bundled portable map engine is not a
   native map. Native bridge failure is an error and must not silently switch
   realization at runtime.

5. **Native bridges must preserve tree shaking and proportional package size.** A bridge may make only the platform code and narrowly scoped support code required by the selected WaterUI features reachable in the final artifact. Do not hide a complete third-party framework or engine behind FFI, reflection, service registration, umbrella dependencies, or broad keep rules and label the result "native"; those boundaries can root the entire dependency graph and defeat R8, linker dead stripping, Cargo feature pruning, and equivalent size optimizations. Unused WaterUI features must remove their Rust code, platform code, resources, and transitive dependencies from the packaged application. Backend dependencies must be feature-granular, and any new runtime dependency requires measured before/after release-artifact size evidence on every affected packaging format. Wrapping the complete ExoPlayer / Media3 stack as an Android native video player is explicitly forbidden for both architectural ownership and package-size reasons.

6. **Cross-platform default appearance is the framework's job, not the view code's.** When a backend renders a primitive, it must read theme tokens (`Foreground` / `Background` / `Surface` / `SurfaceVariant` / `Border` / `Accent` / `MutedForeground` / `AccentForeground`) instead of hard-coding `.label` / `.systemBackground` / `NSColor.windowBackgroundColor` / `UIColor.secondarySystemBackground`. View code calling `.foreground()`, `.background()`, `text("…")` etc. with no extra modifiers must produce platform-correct output. If view code has to reach into a backend to make defaults right, that is a backend bug — fix the backend, do not paper over it in user-facing code.

7. **Asymmetric primitives are documented, not faked.** When platform A has a primitive and platform B genuinely doesn't (e.g. SF Symbols on Apple vs no OS-supplied icon catalog on Android), the primitive is supported on A and **explicitly unsupported on B**. Do not bundle a Material font and pretend it is "system." For portable code, depend on a packaged icon-set crate (`waterui-icons-lucide`, `waterui-icons-material-icon`, `waterui-icons-fontawesome7`). Surfacing the asymmetry as documentation is the right answer; hiding it behind a fallback is not.

8. **Fine-grained reactivity is non-negotiable.** WaterUI uses precise per-`Binding` / `Computed` updates, not SwiftUI-style structural diff. APIs that would force a structural recompute on every state change (e.g. requiring rebuild of an entire subtree to update a single text value) are rejected. New API surfaces must accept signals (`impl IntoComputed<T>`, `impl Signal<Output = T>`, `Binding<T>`) rather than plain values when the underlying state is dynamic. Avoid `Dynamic::watch` / `watch(...)` as a default tool: it rebuilds and replaces the watched subtree, so any state owned inside that subtree is lost. Prefer Vue-like precise updates through signal-aware component inputs, metadata, modifiers, or explicit backend semantic objects.

9. **React-style local state slots are forbidden.** Do not introduce or depend on renderer-provided local state slot mechanisms such as `LocalStateScope`, `LocalStateStore`, `local_binding`, `with_local_binding_factory`, hook-like slot storage, body-position keys, or cursor-indexed state to preserve component-local state across body evaluation. WaterUI is not React: component identity and state must not be inferred from view body call order. Mutable UI state must be explicit `Binding` / `Computed` / `impl Signal` state owned at the correct semantic level and passed through the API or backend semantic object. If existing code needs renderer local slots to work, refactor that component/state model; do not add backend support for the slot mechanism.

10. **Do not change WaterUI foundations without user approval.** Do not modify `core/`, foundational animation/reactivity/layout primitives, or shared backend contracts unless the user explicitly approves that foundation change in the current task. External references are evidence for values, semantics, and behavior; they are not permission to import another framework's abstraction model into WaterUI.

## Repository Boundaries and Distribution

These are the target architecture and acceptance criteria for repository changes. Existing in-tree implementations and backend submodules are migration state, not exceptions to the boundary. A layout description below is not evidence that a migration or release has completed.

- **WaterUI owns the foundations and the unified facade.** Core semantics, public backend contracts, and `waterui-graphics` APIs stay in the main repository. The `waterui` crate is a facade, like `crossbeam`: it re-exports independently maintained crates, including chart, rather than requiring every application to assemble the ecosystem manually. Use direct re-exports, not wrapper types or duplicate implementations. Keep heavyweight components optional through coherent Cargo features; disabling a feature must remove its dependency graph. Direct component imports and facade imports must expose the same types, including with dynamic linking.
- **Self-drawn components are independent versioned crates, not submodules.** SVG, canvas, chart, particle, barcode, and similar components own their implementations, tests, releases, and CI in their own repositories. They consume public `waterui-graphics` APIs and the core/layout/reactivity crates they actually need. Stable integration uses published crate versions. They must not depend on another repository's checkout layout or reach into its source tree for implementation or test resources.
- **Component-specific examples follow their component repository.** For example, the barcode demonstration belongs in `water-rs/barcode`, not WaterUI's `examples/`. Move the complete example, assets, project configuration, and validation responsibility with the component; update workspace membership, CI paths, and links, and remove the main-repository copy once the standalone example works. Use the owning repository's component source and explicit compatible framework dependencies, without relying on the WaterUI checkout layout. Keep examples in WaterUI only when their actual purpose is framework behavior or cross-component integration, not as a way to retain a second component demonstration.
- **Normal dependencies flow from the facade to components to foundations.** A component re-exported by `waterui` must not normally depend back on `waterui`. A backend that consumes the facade cannot also be re-exported by it until that reverse dependency is removed. Repository independence does not itself require a facade re-export of every backend.
- **Concrete backends also have an independent-repository boundary.** Apple, Android, GTK, Hydrolysis, and Dew own their platform implementations and CI; shared backend contracts remain in WaterUI. Apple's separate Git repository supports SwiftPM remote-package distribution and an independent Swift toolchain, but SwiftPM does not require a submodule in the WaterUI repository. Rust backends are not exceptions: replace workspace-only assumptions, cross-directory fixtures, and CLI path assumptions with public package interfaces and explicit version metadata before switching consumers. Hydrolysis can remain the integration-test host as a versioned dependency. Preserve each backend's rendering model; extraction is not permission to redesign shared contracts.
- **Keep local ecosystem checkouts together.** Independent component and backend repositories live under `~/Coding/water-rs/`, not as scattered siblings directly under `~/Coding/`. Create new extracted repositories there too. The WaterUI canonical checkout remains at `~/Coding/waterui`, and active agent workspace slots retain their existing paths. When relocating an existing checkout, preserve its entire Git directory, branches, staged and unstaged work, and update operational path references rather than cloning a second development line.
- **Hydrolysis and its Material 3 theme are two independent repositories.** `hydrolysis` lives in water-rs/hydrolysis and `hydrolysis-m3` in water-rs/hydrolysis-m3; neither is nested in the other and neither is a WaterUI submodule. Each owns its implementation, dedicated examples, assets, tests, CI, and releases. Hydrolysis remains theme-independent and GPU-required; the MD3 package implements public widget/theme contracts without depending on renderer internals. WaterUI's CLI and integration tests select explicit compatible versions of both packages through channel metadata. A change to either is a pull request in its repository and a version bump here, never a crate in this tree.
- **Reserve submodules for inseparable core dependencies such as nami.** Being first-party, needing independent CI, or being distributed from a Git repository is not sufficient reason to add a submodule. Independently distributed components and backends are consumed through explicit compatible package versions or source revisions, not brought back as submodules. Existing non-core gitlinks must be migrated deliberately, not removed before their replacements work.
- **A split is complete only when both development lines are reconciled.** Compare the in-tree implementation with the standalone repository and the artifact actually consumed. Preserve unique fixes, tests, fixtures, documentation, and traceable source history from both sides. Account for local uncommitted work rather than silently overwriting or abandoning it. Verify the standalone build and the consuming framework together before removing the in-tree copy. If the required public foundation APIs are unreleased, prepare compatible foundation and component releases through the release workflow; do not consume an older artifact that loses functionality or maintain a second implementation to hide the mismatch.

## CI Ownership and Framework Channels

- **Repository boundaries also define CI ownership.** Each component/backend repository checks its own implementation and platform matrix, with triggers scoped to affected packages and paths. WaterUI checks its foundations, facade exports, and cross-repository integration. Do not recreate the monorepo by running every extracted crate's implementation suite as a WaterUI workspace member. A moved check must have a verified owner; no platform, feature combination, fixture, or correctness signal may disappear during extraction.
- **The WaterUI dev-push gate is format plus compilation/lint checks, and must finish in under 10 minutes.** Full tests, doctests, examples, expensive platform/feature matrices, coverage, and other complex suites run nightly rather than on the dev-push critical path. Additional PR ABI/security checks remain explicit. Prioritize cache capacity for the frequent compilation gate. Prove the budget with actual Actions wall-clock measurements, including setup and cache overhead; setting a timeout or skipping correctness checks is not proof of success.
- **Framework channels are not Rust toolchain channels.** The CLI must support the following framework selections:

  | Channel | Source and guarantee |
  | --- | --- |
  | `dev` | The integration `dev` branch, resolved to an exact commit; compilation-checked, not test-certified. |
  | `nightly` | An immutable tagged revision whose complete required integration suite passed, with the tested dependency/backend combination. It is not a moving development branch or merely the latest commit of the day. |
  | `stable` | Formal crates.io releases with compatible native-backend versions; publishing remains owned by release-plz. |

- **Promote tested snapshots, never moving inputs.** A nightly run captures one framework commit, its dependency lock, and exact backend/core references. Only after every required check succeeds may the promotion workflow create an immutable nightly tag and GitHub prerelease. Failed, cancelled, incomplete, or older runs must not replace or outrank a newer certified nightly. Keep the last successful nightly usable. Nightly promotion must not publish to crates.io or invoke stable release automation.
- **The CLI owns coherent version selection.** Resolve a channel on project creation or an explicit version change/update, then persist the channel and exact resolved versions. Normal build/run operations must not silently upgrade dependencies or mix a tested framework with independently advancing backend branches. Preserve the tested dependency lock for nightly rather than resolving newer compatible component releases. Stable uses published crates; local-checkout development remains explicit and must not override a requested channel implicitly. If no certified nightly exists, return an error rather than falling back to dev. Put resolution in the CLI library, not in terminal interaction wrappers.

- **Framework revisions own their minimum compatible CLI version.** Declare it in the root `Cargo.toml` under `[package.metadata.waterui].minimum-cli-version`, using a concrete SemVer version. Raise it when a framework change requires newer CLI behavior, not automatically for every CLI release. The library checks local source metadata, resolved `waterui` package metadata, and the requirement persisted with a channel before backend scaffolding or builds. Nightly certification carries the same declaration. Rejection names the installed and required versions and gives an update command for the selected source; it never silently switches framework channels or upgrades the executable. Previously released CLIs without this check still need a one-time manual update.

## Engagement Rules

**Prefer a coherent design over a small diff.** Avoiding overengineering means avoiding unnecessary complexity, not avoiding substantial refactoring.

- When the existing structure or abstraction is the root cause, prefer replacing it with a sound design over accumulating local patches, special cases, or adapter layers. Update affected consumers and remove the superseded implementation rather than maintaining parallel paths.
- Judge a solution by correctness, clarity, and long-term maintenance, not by lines changed. A broad refactor is preferable when it resolves the underlying problem more cleanly; a local fix is appropriate when the design itself is sound.
- Prioritize the integrity of the overall design, including the abstractions and infrastructure it calls for. Avoid defensive programming: express invariants through clear contracts and types, and expose violations directly instead of layering speculative guards, retries, or fallbacks over them. Validate external inputs at real trust boundaries, but do not prematurely handle scenarios that have no plausible path to occurring.
- Do not turn every concern or fix into another permanent regression test. Add durable tests selectively for meaningful, recurring failure risks; consider existing coverage, redundancy, maintenance burden, and cumulative suite runtime.
- Keep clearly one-off diagnostic and validation tests temporary and out of the repository. Workflow configuration changes normally use `actionlint`, focused local checks, and actual Actions runs rather than a permanent CI self-test suite.
- Preserve meaningful product coverage while keeping the suite effective. Prefer extending or consolidating existing tests when appropriate, rather than continually growing the suite with overlapping checks.
- Keep the scope tied to the root problem, including the refactoring needed to solve it, rather than unrelated cleanup. Where architectural approval is required, propose the root redesign directly instead of substituting a patch.

**A bug you find is a bug you fix, even when you did not introduce it.** Do not
route around it, do not leave it for someone else, and do not merely mention it
in chat and move on. Open a GitHub issue for the defect, then land the fix as a
pull request targeting `dev`. This explicitly covers:

- latent failures your own fix unmasks, which is the common case — clearing one
  blocker regularly exposes the next one that was hiding behind it;
- infrastructure that rotted while nobody was looking (a script hardcoding a
  layout that has since moved, a job that has not actually run in months, a
  workflow that silently degraded);
- defects in neighbouring code you had to read in order to do the task.

Say plainly in the commit message, the issue, the PR, and to the user that the
defect was pre-existing, so the diff stays understandable, then fix it. Scope the
fix to the underlying problem, including redesigning the affected structure when
that is the better solution; exclude unrelated changes, not necessary refactoring.

**Fix the root. Never adjust your own code to avoid a bug you just found.** The
tempting move — the one that must not happen — is to leave the defect standing
and quietly steer around it: giving a view an explicit size because the
container that should have sized it collapses, picking different inputs for a
test because the honest ones trip the bug, adding a `.frame()`, a constant, a
retry, a guard, or an extra argument whose only job is to keep the broken path
from being taken. That is a workaround even when the resulting line looks
idiomatic and even when it is one character long, and it is worse than leaving
the bug alone, because the reproduction disappears with it: the next person sees
green tests and working screens over a defect nobody can find any more. If you
caught it, you are holding the only reproduction there is — keep it, and fix
what it points at.

Two consequences worth stating outright:

- **A gallery, snapshot, or example that renders wrong is a bug report.** Do not
  make it render right by construction. Restore the honest version once the root
  is fixed, and keep it as the regression test.
- **"This needs approval" is not a place to stop working.** Foundations
  (Principle 10) still need the user's decision before you change them, so
  present the diagnosis and the exact fix you propose and ask — but present it as
  the question it is, never as a footnote under a change you shipped by routing
  around the defect. If the user tells you to fix it, fix it at the root; a
  second workaround after that answer is a straight violation.

If the correct fix genuinely is large or architectural, surface it with a
concrete recommendation and let the user decide, rather than either silently
expanding the change or quietly abandoning it.

Keep the change set strictly scoped to the task.

- Keep top-level folders semantic and minimal. Do not add generic crate buckets (`crates/`), implementation-detail roots (`internal/`, `facade/`), or top-level folders whose only purpose is a single package manifest. Put crates under the existing domain folder (`components/`, `utils/`, `backends/`, `kit/`, etc. — icon sets live under `components/icon/`) or under `src/` when they describe the root `waterui` package itself. Crate families that share a non-`waterui` prefix belong under one family directory such as `utils/filtrate/`, not as repeated sibling folders like `filtrate-core` / `filtrate-derive`.

- **Before adding a component crate, check whether the workspace already depends on one.** Several components were split into their own repositories and come back in from crates.io, so they are invisible when you search `components/` — `waterui-barcode` (QR and the other symbologies), `waterui-chart`, `waterui-map-gpu`, `waterui-canvas`, `waterui-particle`, `waterui-image`, `waterui-video-gpu`, `waterui-visualizer`, `filtrate`, `shaderloom`, `nami`, `merman`. Read `[workspace.dependencies]` in the root `Cargo.toml` and look in `examples/` for a directory named after the feature; both name the component that already exists. Extending one of those means a change in its own repository and a version bump here, never a second crate in this tree — and a QR-only type beside `Barcode` would be the parallel-type mistake Principle 1 rules out, since the symbology is an attribute of one semantic component.
- Do not drag unrelated files into the diff.
- Do not run workspace-wide formatters or refactors such as `cargo fmt --all`, bulk codemods, or broad search-replace when the task only targets a few files.
- Prefer file-scoped formatting and verification on the exact files you intentionally changed. For direct Rust file formatting, never run bare `rustfmt`; pass the workspace edition explicitly, for example `rustfmt --edition 2024 path/to/file.rs`, so rustfmt does not parse this Rust 2024 workspace as Rust 2015 and does not module-walk into unrelated files.
- Do not run multiple `cargo` commands in parallel. It only creates lock contention and provides no benefit in this repository.
- Do not hardcode versions, repository URLs, package sources, filesystem paths, or other environment-derived constants just to ignore real complexity. If a value has a real source of truth, derive it from metadata, build inputs, repository structure, or runtime context instead of freezing a literal.
- Do not add blind timing workarounds such as fixed sleeps, fixed-duration `RunLoop` waits, or arbitrary retry delays to "probably" wait for readiness. Wire the code to the real readiness/completion signal. If synchronous code must bridge to async readiness, keep driving the relevant event loop only until that concrete readiness condition completes.
- **Never wait silently for more than 1 hour.** The prompt cache TTL is 1 hour; a longer silent gap costs a full re-read of the session. Any waiter or monitor that can run past an hour must emit an event within ~50 minutes (a timeout it re-arms on, or a heartbeat line from the waiter itself), the same rule covers the end of a turn with work still pending, and every waiter is killed the moment its purpose is served.
- **`std::thread::sleep` is banned in tests. There is no exception for "just advancing an animation".** The animation clock advances when frames are pumped, not when wall-clock time passes, so a bare sleep freezes it: every deferred step is then applied at once on the next snapshot, and a capture meant to show a transition mid-flight silently shows its end state. The test still passes and the PNG still looks plausible, which is what makes this one dangerous. To sample a phase, pump: `OffscreenApp::pump_for(Duration)`. To wait for a condition, wait on the condition (`Query::wait_for_existence` and friends). The only sleep that belongs anywhere is the per-frame pacing *inside* a pump loop.
- Check `git status --short` before and after formatting or codegen steps. If unrelated files appear, stop and narrow the command instead of continuing with a polluted diff.
- Only use repo-wide formatting or sweeping rewrites when the user explicitly asks for them or the task genuinely requires touching the whole workspace.
- Follow **CI Ownership and Framework Channels** above, which supersedes the test-heavy dev gate from #379. The fast gate is tracked in #458; immutable nightly promotion and CLI channel selection are #460 and #461. Preserve the full nightly suite and failure reporting while moving implementation-specific checks to their owning repositories. Do not treat a scheduled workflow as a certified distribution until its promotion and version-locking contracts are implemented.
- Workflow files under `.github/workflows/` may be changed WITHOUT asking when the change is a pure performance optimization that preserves coverage: cache keys and `save-if`/`cache-targets` tuning, job splitting or reordering, timeouts, runner sizing, moving a non-gating leg off the critical path onto a schedule, or adding a fast lane. A slow pipeline is a defect to fix, not a fact to endure. What still requires explicit authorization is any DEGRADATION: removing or skipping tests, dropping a platform or feature combination, loosening a lint gate, disabling a check, or trading correctness signal for speed. When in doubt about which side a change falls on, ask.
- GitHub Actions workflows should stay minimal and declarative. Do not put heavy release logic, repository analysis, packaging validation, or hand-rolled orchestration scripts into workflow YAML when a maintained community tool can own that behavior.
- Cross-Backend Regression is a CI pipeline concern, not user-facing README documentation. Keep references to it in CI/developer-maintainer context rather than public product docs.
- Prefer maintained community actions and purpose-built tools over custom shell/Python scripts in workflows. Release publishing should be delegated to `release-plz`; only the CLI binary prebuild/release-asset handoff is expected to require extra workflow glue.
- A release PR carries one edit release-plz cannot make: `[package.metadata.waterui-scaffold]` in `cli/Cargo.toml` holds the versions a *published* CLI scaffolds projects against, and a published CLI has no workspace to read them from. `cli/build.rs` fails the build with the exact lines to write whenever they fall behind, so a release PR that bumps a crate is red until they are pushed to its branch. That is the point: a CLI released with stale literals hands every new project a framework a release or two old (#548).
- Do not patch around repository-state problems by adding workflow preflight scripts or CI workarounds. Fix the source tree, manifests, submodules, or release configuration at the real source of truth.
- Do not add crate-level, file-level, or module-level `allow` attributes to skip lint failures during cleanup. Treat lint as code-quality feedback and fix the underlying code, API shape, docs, or type invariants instead.
- If a lint is a genuine false positive or conflicts with the intended architecture/readability, prefer a narrowly scoped item-level `allow`/`expect` with a concrete reason over contorting the code to satisfy the lint. WaterUI is a main-thread UI framework, so UI-local `spawn_local` futures that capture non-`Send` view state are a valid example. Do not use broad lint exceptions, and do not add exceptions without evidence.
- MANDATORY for every agent (Claude Code, Codex, and any future agent): **the canonical checkout is read-only.** Investigate in it freely; every edit, however small, happens inside an agent workspace, because `finish_workspace.sh` refuses to run while the canonical tree has uncommitted changes and a one-line stray edit blocks every other agent from merging. The tooling is committed at `.claude/skills/waterui-agent-workspace/` and its `SKILL.md` is the reference for the lifecycle: `scripts/create_workspace.sh <task-slug>` hands out a warm slot at a fixed path, `scripts/sync_workspace.sh` refreshes it when canonical advances, and `scripts/finish_workspace.sh` merges back under the integration lock and releases the slot. `finish_workspace` is a script, never a hand-rolled fast-forward. One active workspace per session; tasks are serial.
- Use the `waterui` skill only when authoring WaterUI app/example code or checking public user-facing API usage; it is distinct from `waterui-agent-workspace` (workflow tooling for agents).
- The repo-local `.claude/skills/waterui/SKILL.md` is for WaterUI users. Update it only when a user-facing public authoring pattern, API usage rule, or app-level CLI usage changes.
- "Visual test" in this repository means the agent reads the generated image directly with its own vision capability. Heuristic image checks are forbidden, including changed-pixel counts, opaque-pixel thresholds, bbox approximations, dominant-color checks, brightness checks, non-uniform checks, and similar proxy code.
- Before writing any new image/gallery/snapshot export code, search for and reuse the existing `waterui-testing`, preview, showcase, GPU snapshot, or filter gallery infrastructure. Do not add ad-hoc gallery examples, scripts, or binaries unless the user explicitly asks to create or extend that infrastructure.
- For filter visual review images, the canonical reusable infrastructure is `cargo nextest run -p filtrate --lib -E 'test(gpu_export_filter_gallery_images)' --no-capture`, which exports PNG files to `/tmp/waterui_filter_gallery/`. Use this path to show filter outputs instead of creating a new gallery generator.
- Keep operational agent guidance in this file, not in product documentation or public change narratives. Write issue, PR, and commit text directly as project statements: no assistant signatures, generated-by footers, session links, or third-person delegation/approval narration. Preserve technical tool names and paths when they are the actual subject, and preserve contributors' factual findings and verification results.
- Keep `.claude/skills/waterui/SKILL.md` strictly user-facing. If information is primarily for agents or maintainers rather than app authors using WaterUI, it belongs in `AGENTS.md` or implementation docs, not in the user-facing skill.
- `.claude/skills/waterui/skill_snippets/` is the compile gate for `.claude/skills/waterui`: every rust fence in the skill is transcribed there (verbatim modulo rustfmt, with loudly-marked glue) and CI compiles it. When you change a skill code snippet, regenerate the matching module following the conventions in that crate's README. Its `#[waterui::test]` / `#[waterui::bench]` transcriptions sit behind the non-default `compile-gate-tests` feature: CI compiles them with `cargo check -p skill_snippets --all-targets --features compile-gate-tests`, and they must never be executed — they address elements that do not exist, by design.
- `waterui-testing` is based on the Hydrolysis accessibility tree, not native platform accessibility. Prefer `waterui-testing` for UI component coverage, and treat it as both an interaction test and an accessibility-correctness test.
- Every UI component is expected to produce a meaningful accessibility tree. If a component cannot be covered by `waterui-testing`, treat that as a bug to fix rather than a gap to paper over.
- `GpuSurface::new(renderer)` owns one `GpuView` instance for that surface lifetime. `GpuView::setup()` is where persistent GPU resources for that renderer instance belong. Do not move renderer state into hidden shared caches just to survive `GpuSurface` teardown or parent rebuild.
- For text APIs, use `text()` for static text and `text!` for reactive formatting. Do not use `watch()` to build reactive text when `text!` or signal-taking APIs already express the dependency directly.
- A **dynamic set of views** is a collection, not a `watch`: render it with `ForEach`/`List` over a reactive collection (`nami::collection::List`, `Identifiable` items) so membership changes diff by id. `watch(binding_of_vec, …)` rebuilds and re-dispatches the whole subtree on every change (and may escalate to a full-window structural rebuild) — that is the watch-abuse Principle #8 forbids. Authoring layer uses `ForEach`/`List`; the backend has `get_id`/`watch`/`get_view` to render it incrementally.
- A **window overlay layer** (snackbar/toast/dialog host) must fill the window — wrap it in `AbsoluteLayout` (`StretchAxis::Both`, hands every child the full window bounds), never a content-sized `ZStack`. The window root composes `zstack((content, overlay, …))` and places a content-sized overlay by its intrinsic size, so edge-anchored children only land correctly in a small window and mis-anchor/vanish when the window is large or resized. A constant-size full-window layer also keeps reactive membership updates from escalating to full-window rebuilds.
- Do not write `waterui::text!`. Always import the macro first, then use the short `text!` form.
- Do not inline absolute paths like `::waterui_core::views::ForEach` inside macro bodies or expanded code. Bring the names into scope with `use ...;` at the call site (or in the surrounding module) and reference them with bare identifiers — `ForEach`, `Collection`, `Identifiable`, `View`. The same applies to plain function/type usage: import first, use bare names. Long absolute paths add visual noise and break the look of declarative WaterUI code.
- Do not add `anyhow` as a direct dependency in any `Cargo.toml` in this workspace. The error type is re-exported as `waterui_core::Error`; reach for that re-export when implementing traits whose associated error is `anyhow::Error` (e.g. `Extractor`). `thiserror` and other error-construction utilities are unaffected.
- **Whoever owns the main loop supplies the `LocalExecutor`.** Every WaterUI host
  already has one — winit (`WinitMainThreadExecutor`), GTK
  (`GtkMainThreadExecutor`, via `glib::idle_add_local_once`), headless
  (`HeadlessMainThreadExecutor`), dew (`embedded_executor::install()` plus a
  per-frame `tick()`). Give `try_init_local_executor` an executor bound to that
  loop; never hand it `native_executor::NativeExecutor`. On non-Apple targets
  `NativeExecutor` delegates to the polyfill, whose `spawn_main_local` asserts it
  runs on the thread registered by `start_main_executor` — a blocking, never-
  returning entry point that a loop-owning host must not call, because it would
  declare some unrelated thread "main" while `MainThreadBound`, layout and the
  GPU surface all live on the loop thread. `NativeExecutor` remains correct for
  `try_init_global_executor`, which needs no main-thread affinity. The mistake is
  made at install time but only panics at the first `spawn_local`, so it is worth
  checking explicitly whenever a new host or test harness is added.
- Measurement caching is the `SubView`'s responsibility, never the `Layout`'s. The `Layout` trait deliberately has no cache — containers probe children freely with many proposals — so any caching (text shaping above all, which **must** cache) lives in the `SubView` implementation. Layout measurement is single-threaded by contract (running on whichever thread drives layout), so a `SubView` is neither `Send` nor `Sync` and its cache may be a plain `RefCell` (as in `MemoizedSubView`). Parallelism belongs in batched renderer pre-passes, not per-container measurement loops. Do not add caching to `Layout`.

<important>
    For rust: YOU CANNOT USE println, use tracing::debug!() instead for debug output.
    For swift: YOU CANNOT USE print(), use Logger instead for debug output. It uses `dev.waterui` as the log subsystem.
    For kotlin: YOU CANNOT USE println(), use Log.d() instead for debug output.

    Note that debug output will only appear if the CLI is run with --logs debug flag.

    For application creation, builds, previews, and execution, ALWAYS use the water CLI (water create, water build, water preview, water run, etc.). Do not bypass its application workflow with direct native-tool invocations.

    Standalone crate/backend-package verification uses the package's own toolchain: Cargo for Rust, SwiftPM for Swift packages, and Gradle for Android packages. This does not authorize hand-scaffolding an application or bypassing water for application deployment. If an application workflow requires direct adb/xcodebuild/other tool use because water lacks the capability, propose adding that capability to the CLI.

    Never hand-create or manually scaffold project/app structure. Always use `water create` (or existing generated project files) as the source of truth.
    For monorepo examples/playgrounds in local dev mode, `Water.toml` must explicitly set `waterui_path = "../.."` to force local backend usage and avoid remote backend resolution.
</important>

<important>
- Follow fast fail principle: if an unexpected case is encountered, crash early with a clear error message rather than fallback.
- Utilize rust's type system to enforce invariants at compile time rather than runtime checks.
- Prefer structs, traits, and generic abstractions over enums and type erasure when they express the intended model.
- Public traits expose the friendliest signature even when it is not object-safe (`-> impl Future`/`-> impl View`, generic methods, RPITIT). When dynamic dispatch is needed internally, do NOT degrade the public trait: add a private object-safe shim trait (`XxxImpl`) with a blanket `impl<T: Xxx> XxxImpl for T`, and store `Box<dyn XxxImpl>` behind a public wrapper type (`AnyXxx` / `ViewRenderer`-style). Type erasure is an implementation detail, never the user-facing API shape (see `core/src/ui/view_renderer.rs` for the canonical example).
- The C ABI cannot carry generics, so every native/config type stores **erased** selection and item state: `Binding<Id>`, `Binding<Option<Id>>`, `Computed<Vec<PickerItem<Id>>>`. That erasure is deliberate and correct at that layer — do NOT report it as a design flaw, and do NOT try to make the FFI representation generic. Keep it *below* the authoring layer instead: the public constructor stays generic over the app's own type and erases through `Mapping<T>` (`core/src/foundation/id.rs`), which assigns stable `Id`s and maps them back with `to_data`. Canonical pairs are `Picker::new<T>` → `PickerConfig`, `NavigationSplitView::new<T>` → `NavigationSplitLayout`, and `Tabs::new<T>` → `TabsLayout`. The bug to look for is the opposite one: a type that is simultaneously the authoring API and the raw view (`raw_view!` + `ffi_view!` on the same struct) leaks `Id` into app code and forces callers to write `Id::try_from(1)` (no current component has this defect). Fix that by adding the generic constructor, never by changing the FFI type.
- Put shader to a separate file rather than embedding as string literal. Same for large text assets.
- Do not write duplicated code. If you find yourself copying and pasting code, consider refactoring it into a shared function or module.
- Preserve the selected renderer's contract: Hydrolysis is GPU-required; Dew deliberately uses CPU rasterization for constrained devices. Do not add a CPU fallback to Hydrolysis or force GPU dependencies into Dew's lean graph.
- You are not allowed to revert or restore files or hide problems. If you find a bug, fix it properly rather than working around it.
- Do not leave legacy code for fallback. If a feature is deprecated, remove all related code.
- No simplify, no stub, no fallback, no patch.
- Do not use `pkill` blindly in scripts, as it may kill other important processes. Instead, track PIDs of spawned processes and kill them specifically. For instance, `pkill -9 -f "WaterUIApp" 2>/dev/null` is not allowed.
- Do not clean cache blindly
- Never disable `sccache` under any circumstance (do not set `WATERUI_DISABLE_SCCACHE=1`, and do not bypass `sccache` via `RUSTC_WRAPPER=`), because disabling cache causes storage usage to explode.
- Never read back GPU render targets/textures to CPU memory in runtime render paths. This violates GPU-first architecture and causes severe performance degradation.
- Do not use `git checkout` to back out changes, as it can lead to loss of work
- Import third-party crates instead of writing your own implementation. Less code is better.
- Do not create custom Cargo target directories (for example, `CARGO_TARGET_DIR=/tmp/...`) in this monorepo. Always use the repository's default `target/` directory.
- `GpuSurface` supports offload/offscreen rendering. When developing any `GpuRenderer`-based component, you must use offload/offscreen rendering for visual testing.
- CI is expensive, please read full error message if CI fails. Do not blindly push commits to trigger CI again before fixing all problems you learnt.
- For public API design, follow this repository style consistently: `Type::new(...)` is the general constructor, while free function constructors such as `button(...)` are ergonomic convenience entry points. Do not introduce parallel APIs like `Type::custom(...)` when `Type::new(...)` already covers the general case.
- Keep the constructor split explicit in API design and documentation:
  - `Type::new(...)` is the general constructor and should accept the most general shape that the component can render.
  - Free function constructors like `button(...)` are ergonomic convenience entry points and may accept narrower semantic input types for better defaults.
  - Example: `Button::new(...)` should remain the general constructor for arbitrary label views, while `button(...)` is the ergonomic constructor that accepts `IntoLabel` so literals, i18n-friendly text, and default accessibility semantics compose naturally.
  - For semantic text and label APIs, prefer `IntoText` / `IntoLabel` over raw `impl View` so string literals naturally enter the i18n-aware semantic text pipeline. Only accept `impl View` when the API is intentionally for arbitrary visual composition rather than semantic text or labels.
</important>

## Build Commands

```bash
# Install CLI from source (required for `water run` to work)
# You must reinstall cli to path after modifying it if you wanna debug it.
cargo install --path cli

# Build CLI for development (faster iteration, but not in PATH)
cargo build -p waterui-cli

# Build entire workspace
cargo build --workspace

# Run tests (nextest is the default runner; see "Testing Patterns")
cargo nextest run --workspace

# Run tests for specific crate
cargo nextest run -p waterui-core
cargo nextest run -p waterui-cli

# Run workspace doctests separately from nextest
cargo test --doc --workspace

# The web view bridge's JavaScript unit suite is required by nightly.
# Also run it locally after touching components/platform/webview/src/js/
bun test components/platform/webview/tests/js/

# Generate FFI C header (after modifying ffi/ APIs), never write C header by hand.
# The generator is its own crate so building it costs cbindgen, not the framework.
cargo +nightly run --manifest-path ffi/generator/Cargo.toml

# Build Apple backend
cd backends/apple && swift build

# Build Android runtime (the wrapper lives in the submodule, not at the repo root;
# `local.properties` is gitignored, so point Gradle at the SDK yourself)
cd backends/android && ANDROID_HOME=$HOME/Library/Android/sdk ./gradlew runtime:assembleDebug

# Run demo app (after creating a project)
water run --platform ios
water run --platform android
water run --platform linux --backend hydrolysis

# Create a playground for quick experimentation
water create "My Playground" --mode playground

# Preview a view function (renders to PNG without running full app)
water preview my_view --platform macos --path ./app --output preview.png
```

## Playground mode

Playground mode allows CLI to delegate the detail of backend integration to the user, for instance, you cannot touch Xcode project directly in playground mode. Playground mode is recommended by default. All waterui project in this repo is in playground mode.

## Preview System

The `#[preview]` macro enables instant view rendering without running the full app:

```rust
#[preview]
fn my_card() -> impl View {
    text!("Hello Preview!")
}
```

Symbol format: `waterui_preview_{crate_name}_{fn_name}` (crate name included to avoid conflicts).

The preview system:
1. Builds the project as a dylib
2. Launches a preview app that loads the dylib
3. Renders the view to PNG via native rendering pipeline
4. Supports macOS, iOS Simulator, and Android

## Architecture Overview

WaterUI is a cross-platform reactive UI framework with both native platform bridges and self-drawn backends. Shared semantics and public rendering contracts are independent of the selected realization.

### Core Data Flow

```
Rust View Tree → Public view/backend contracts
  → FFI (C ABI / JNI) → Apple / Android native backend → Platform UI
  → Rust backend → GTK native widgets or Hydrolysis GPU / Dew CPU rendering
```

### Crate Structure

- **`waterui`** - Main crate, re-exports components and provides `prelude`
- **`waterui-core`** - Foundation: `View` trait, `Environment`, `AnyView` type erasure, reactive primitives (`Binding`, `Computed`)
- **`waterui-ffi`** - C FFI layer bridging Rust to native backends; `export!()` macro generates entry points

### Component Libraries (`components/`)

- `layout` - HStack, VStack, ZStack, ScrollView, Spacer
- `controls` - Button, Toggle, Slider, Stepper, Picker, Progress
- `text` - Text, styled text, fonts, markdown
- `form` - Form builder with `#[form]` derive macro
- `navigation` - Navigation containers, TabView
- `media` - Video/audio playback
- `graphics` - Canvas drawing primitives

### Backends (current checkout layout)

The paths below describe the existing `backends/` layout, not permanent repository ownership. Follow **Repository Boundaries and Distribution** when extracting implementations or resolving their versions.

- **`apple/`** - Currently a git submodule; independently distributed Apple Swift package
- **`android/`** - Currently a git submodule; independent Android Views + JNI Gradle project
- **`dew/`** - Self-drawn CPU renderer (`vello_cpu` sparse-strip) - experimental. The embedded / constrained-device renderer (see "Rendering backend philosophy" below)
- **`gtk/`** - GTK4 backend

The high-end self-drawn renderer is not in this directory. `hydrolysis`
(water-rs/hydrolysis, #480) and its Material 3 widget theme `hydrolysis-m3`
(water-rs/hydrolysis-m3, #481) are consumed from crates.io. The versions the
workspace builds against are the `[workspace.dependencies]` entries in the
root manifest, and `[patch.crates-io]` there resolves the framework crates
they name to this tree so the graph carries one copy of each.

#### Rendering backend philosophy: Hydrolysis vs Dew (self-drawn renderers)

WaterUI ships two self-drawn (non-native) renderers at deliberately opposite design points. They share `waterui-core`, reactivity, layout, and text, and diverge **only** in their render/flush strategy. Do not converge them, and do not port one's strategy onto the other — the divergence is the point. When touching either renderer, keep the change consistent with its half of this contract; a change that makes Hydrolysis frugal or Dew heavyweight is wrong by design.

**Hydrolysis — the game-engine renderer (high-end, future-facing).**
- GPU-first and GPU-required: rendering goes through Vello on `wgpu` with compute-shader support mandatory; there is no CPU rasterization path (`use_cpu: false`). Never add a CPU fallback or read GPU targets back to CPU in runtime rendering paths; offscreen test/snapshot export is a separate verification path.
- Whole-scene redraw whenever it draws at all, like a game engine. There is intentionally **no** dirty-rectangle / partial-region / damage tracking, and there must not be. A frame is never partially redrawn: no region invalidation, no "only this widget changed so only repaint that rect", no damage accumulation. Do not add any of it.
- The scope of a frame is all-or-nothing; *whether* to run one is a separate question, and Hydrolysis is free not to. The window pump is a two-state machine (`FrameMode` in the renderer's `src/runner/window.rs`): `Idle` does no work at all, and `Refresh` runs the full pass (relayouts the retained tree and re-encodes it). Every awake frame runs layout so the presented scene can never be stale against it. A window with nothing to show does not burn a frame. This is not damage tracking — every frame that *does* run still redraws the whole scene. Do not conflate the two: adding partial-region painting is forbidden, while skipping an idle frame is the design.
- Targets high-end modern devices and high frame rates (120fps and above). High-refresh must be requested **explicitly** per platform (opt into ProMotion / high-refresh display links), not left to incidental vsync. Do not introduce a hard frame cap.
- Designed to exploit modern hardware fully: modern GPU compute **and** multi-core CPU. Parallel scene building / rasterization across cores is part of the intended design; single-threaded execution is a gap to close, not the target. Do not assume or hard-wire single-threaded rendering.

**Dew — the embedded renderer (constrained, resource-frugal).**
- CPU-first; GPU is optional. The default and common path is pure-CPU rasterization (`vello_cpu` sparse-strip). It must run on MCU-class microcontrollers with no GPU and no full-resolution framebuffer.
- Dirty-area rendering is the core architecture, not an optional optimization: only changed regions are re-rasterized, sliced into horizontal bands, so peak pixel memory is one band — never a full frame. Do not introduce full-frame redraw into Dew.
- Modest, power-frugal frame rates: 30/60fps (the runtime ticks at ~16ms). Do not target 120fps here.
- Lean, feature-gated dependency graph: firmware builds strip `gpu`/`widgets`/`gestures` and other heavy deps (`default-features = false`). Dew is `std`-based via its embedded RTOS, not bare-metal `no_std`. Do not pull GPU / `wgpu` / heavyweight crates into Dew's firmware graph.

### CLI (`cli/`)

The `water` CLI orchestrates builds across platforms:

- `water create` - Scaffold new project (supports `--mode playground` for quick experiments)
- `water run` - Build and deploy to device/simulator
- `water build --platform <platform>` - Build the project for the selected platform and backend
- `water package` - Package built artifacts for distribution
- `water clean` - Remove build artifacts
- `water doctor` - Check development environment
- `water devices` - List available devices and simulators

**CLI Architecture Notes:**
- Entry point: `cli/src/terminal/main.rs` - Uses `clap` for parsing, `smol` async runtime
- Commands in `cli/src/terminal/commands/` - Each command is async and returns `Result<()>`
- Platform abstraction: `TargetPlatform` enum in `cli/src/platforming/platform.rs` and `Backend` trait in `cli/src/platforming/backend.rs` implemented by `AppleBackend`, `AndroidBackend`, `Gtk4Backend`, `HydrolysisBackend`, and `Esp32Backend`
- Shell output: `cli/src/terminal/shell.rs` - An explicit `Shell` instance passed to commands, with human-readable (ANSI) or JSON modes

Note: `/terminal/*` (waterui-cli binary) only provide a friendly interface for CLI commands. All real logic should be implemented in the waterui-cli library part.

### FFI Contract

Native backends call into Rust via:

1. `waterui_init()` - Initialize runtime, returns Environment pointer
2. Theme installation (recommended):
   - `waterui_theme_install_color_scheme()` (light/dark)
   - `waterui_theme_install_color()` (slot-based colors)
   - `waterui_theme_install_font()` (slot-based fonts)
3. `waterui_app(env)` - Hand the environment to the app and get its root view tree
4. Render loop: `waterui_view_id()` to identify view type, then either extract data (`waterui_force_as_*`) for raw views or recurse via `waterui_view_body()` for composite views

Raw views are leaf components (Text, Button, etc.) that map to native widgets. Composite views have a `body()` returning other views.

### Reactive System

Uses `nami` crate for fine-grained reactivity:

- `Binding<T>` - Mutable reactive state
- `Computed<T>` - Derived reactive values
- Views automatically update when reactive values change

<important>
    WaterUI uses precise fine-grained reactivity with Vue-like reconstruction semantics. A component's `.body` may be heavy and may perform one-time initialization for that component instance. After initialization, dynamic behavior is expected to be driven precisely through `Binding`, `Computed`, and other `impl Signal` inputs. `Dynamic::watch` / `watch(...)` directly replaces the watched subtree when the signal changes, which loses state owned by that subtree. Treat it as an exceptional primitive, not normal reactive UI authoring. Prefer signal-aware APIs, metadata, modifiers, or explicit backend semantic objects that update the exact dynamic field without recreating component identity. If a component is recreated by control flow such as `when(...)`, `watch(...)`, or other parent-driven reconstruction, losing that component instance's local state is expected and correct because a new instance is being initialized. Do not preserve component-local state across rebuilds unless that state is explicitly owned at the correct reactive level.
</important>

<important>
    React-style local state slots are architecturally banned. Do not use `LocalStateScope`, `LocalStateStore`, `local_binding`, `with_local_binding_factory`, hook-like slot storage, body-position keys, or cursor-indexed state as a WaterUI component state model. Do not fix a crash by teaching a backend to support this mechanism. The correct fix is to move the state into explicit `Binding` / `Computed` / `impl Signal` inputs or into a backend-owned semantic object whose lifetime is independent of Rust body evaluation order.
</important>

<important>
    You are not allowed to use `.get()` on Signals/Bindings directly in view body functions, as it breaks reactivity tracking. Instead, use zip and map combinators to derive new Computed values that depend on multiple signals.
</important>

### View Trait

```rust
pub trait View: 'static {
    fn body(self, env: &Environment) -> impl View;
}
```

### Application Entry Point Pattern

```rust
pub fn app(env: Environment) -> App {
    App::new(main, env)
}

pub fn main() -> impl View {
    // Return your root view
}

waterui_ffi::export!();  // Generates FFI entry points
```

## Key Development Notes

- Rust edition 2024; the supported toolchain floor lives in `rust-version` in the root manifest, not here
- Workspace lints enforce strict clippy rules including pedantic/nursery
- Existing Apple/Android gitlinks describe the current checkout; use the repository-boundary policy above for the intended distribution model.
- Use the isolated workspace workflow described under Engagement Rules; never use `git worktree`. Keep at most one active workspace per session.
- Edit retained submodules in that workspace on their matching topic branches. Preserve other sessions' work and use the workspace integration tooling rather than switching or rewriting their checkouts.
- The FFI header `ffi/waterui.h` is checked into version control; CI verifies it's up-to-date; **never write C header by hand**
- Add FFI exports and native bridges only when a component requires a genuine native primitive. Pure Rust compositions and self-drawn components reuse existing public contracts without inventing new C-ABI types (Principles 2 and 4).

### Testing Patterns

- Most tests use `#[cfg(test)] mod tests` pattern
- **Use `cargo nextest run`, not `cargo test`.** This workspace is large and
  `cargo test`'s single-process-per-binary harness is painfully slow on it.
  Install once with `cargo install cargo-nextest --locked`.
  - Run workspace tests: `cargo nextest run --workspace`
  - Run a specific crate: `cargo nextest run -p <crate-name>`
  - Run one test: `cargo nextest run -p <crate-name> -E 'test(<name>)'`
  - Show output from passing tests: `--no-capture` (nextest's spelling of
    `-- --nocapture`; it forces serial execution, so scope it with `-E`)
- **Use `cargo test` for doctests and custom main-thread harnesses.** Run
  `cargo test --doc --workspace` for workspace doctests. A `harness = false`
  native integration binary that must own the real process main thread, such
  as the CEF or WKWebView real-engine tests, uses its explicit
  `cargo test -p <crate> --test <target>` command and required features.
- nextest runs **each test in its own process**. A test that relies on a
  sibling's initialization of process-global state is order-dependent; fix
  that assumption rather than switching runners to preserve shared state.
- **The web view bridge has a JavaScript unit suite that Cargo does not discover.**
  `components/platform/webview/src/js/{bridge,state,eval}.js` is injected into
  pages by the backends. If you touch that directory or the bridge envelope/state
  protocol, run:

  ```bash
  bun test components/platform/webview/tests/js/
  ```

  Nightly must run this suite as a required check, include failures in its
  report, and prevent certified-version promotion when it fails (#466).
  Keep the local check as well; moving webview to another repository must
  preserve the suite and its CI ownership. Passing Rust tests or real-engine
  integration tests does not replace this unit suite. It covers regressions
  that previously shipped behind green Rust checks: replies crossing as
  base64, a frozen `waterui` object breaking `state`/`watch`, and integers
  past 2^53 losing low bits in either direction.
- The CLI has unit tests in both its library modules and terminal commands. Run scoped checks with `cargo nextest run -p waterui-cli`, narrowing with `-E` for the affected behavior.
- Use `tracing::debug!` and `water run --logs debug` for debugging runtime issues

### Error Handling

- All command functions return `Result<(), eyre::Report>` for rich error context
- Custom error enums use `thiserror` derive macro
- Shell provides `success!()`, `error!()`, `warn!()`, `note!()` macros for user feedback
