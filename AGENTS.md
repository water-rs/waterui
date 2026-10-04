# AGENTS.md

This file provides guidance to coding agents (Claude Code, Codex, and friends) when working with code in this repository. `CLAUDE.md` is a symlink to this file.

<important>

**Finding a problem → GitHub issue. Solving a problem → pull request to `dev`.** Never push `dev` or `main` to origin. `main` is the release branch; `dev` is the integration branch; both require a pull request. Merging is the user's decision. Releasing is the user's decision too: never ask for a release, a tag, or a publish as a condition of your work — consume unreleased changes from another repository through a git pin to the exact commit on its `dev` (see **Repository Boundaries and Distribution**).


**A force-push without the user's authorization is forbidden.** `git push --force`, `--force-with-lease`, a `+refspec`, and anything else that moves a remote branch to a commit that does not descend from what it held all discard whatever another agent, another session, or the user put on that branch since — this is how a merged pull request was dropped from a stack base before the stack landed. The only force-push an agent may make on its own is to a topic branch it created in the current task, that no other pull request uses as its base, and that nobody else has pushed to, and it is made with `--force-with-lease=<branch>:<the sha you last pushed>` so it fails if that ever stops being true. When the lease fails, read what the other push added and build on it; never override it. Rewriting a branch that is the base of another pull request, a branch another session is working on, or any branch you did not create requires the user's explicit authorization in the current conversation. Bringing a branch up to date with `dev` in those situations means merging `dev` into it, which adds commits and never removes any.

1. When you find a problem — a bug, a broken gallery or snapshot, a rotting workflow, a missing primitive, a design gap — open a GitHub issue with `gh issue create`. The issue is the source of truth; do not keep the finding as a chat-only note. Each issue is one self-contained technical task: a single defect or a single implementable change that can be understood, assigned, and merged on its own. Do not file umbrella issues, roadmaps, or sequenced slices. Do not use the word "phase" (or equivalents such as "stage", "part N of M", "step 1/2/3") in the title or body — an issue is not a chapter of a plan. The one exception is a request with genuinely separable parts: file one parent issue that carries the request's intent and exists only to group its sub-issues — no implementation hangs off the parent, and each leaf sub-issue is itself self-contained and lands through its own PR. Assign the earliest open version milestone to every issue you open, unless the work clearly belongs to a later release.
2. Land the fix on a topic branch. Never `git push origin dev`. Publish the topic branch with `git push -u origin <branch>`.
3. Open the pull request with `gh pr create --base dev` and link it to the issue (`Fixes #N`). One PR solves one issue.

A `dev` → `main` merge publishes everything `dev` has accumulated — including every breaking change — and each merge to `main` triggers a crates.io release. Never use a `dev` → `main` merge to transport a fix intended for the current release line (release plumbing, backend pins, scaffold versions): branch the fix off `main`, open the pull request back to `main`, then back-merge `main` into `dev` to keep `dev` in sync. A `dev` → `main` merge happens only at a deliberate version boundary, and only on explicit user instruction.

Until 1.0, that boundary is the product's major-release step: a new `0.x` minor ships every three months, while patch releases are unbounded. `.github/workflows/version-boundary.yml` enforces the "explicit instruction" mechanically: a pull request to `main` that contains breaking changes (conventional-commit `!` markers or `BREAKING CHANGE` footers) is blocked unless it carries the `release` label, which records that a human intended this merge as the version boundary.

Make sure no warnings or errors are introduced in the codebase. If you encounter a warning or error, fix it before committing. Do not ignore warnings or errors, clippy warnings included.
</important>

## Writing Issues and Pull Requests

Write issues, pull requests, reviews, and commit messages the way a maintainer of an open-source project writes them: for a contributor who has never seen the conversation, the session, or the branch history behind them.

- **Use the template.** `.github/ISSUE_TEMPLATE/` holds the forms for a bug, a feature, a maintenance task, and a performance problem. Pick the one that fits. `gh issue create` cannot fill an issue form, so write the body with the form's fields as `##` headings, in the form's order. Leave out the optional fields you have nothing for, and apply the form's label. Titles follow the forms' `<area>: <what>` convention. Pull request descriptions follow `.github/pull_request_template.md`.
- **An issue states the problem, not the work log.** It says what is wrong or missing, why it matters, the scope, and when it is done. When a design has been decided, the issue may state that design as the proposal. It does not carry implementation step lists, checklists of tasks, or agent instructions. It does not carry review findings, measurement dumps, or progress reports. Findings and results go in comments. The plan for a change lives in its pull request. Review conclusions go in the review.
- **A pull request explains the change to a reviewer.** Say what changes and why, link the issue with `Fixes #N`, and state in a line or two per item how it was verified. Link CI runs and captures instead of pasting logs. Do not narrate the change commit by commit. Do not restate the issue.
- **Write as the project.** No assistant signatures, generated-by footers, session links, model or agent names, or third-person narration of who delegated or approved what. No dates or plan labels the content does not need. Cite evidence by commit, run id, or issue number. Keep tool names and paths when they are the subject. Keep contributors' factual findings and verification results.
- **Keep the record current.** When an issue's description goes stale, correct it in place rather than appending a correction. Use GitHub's own relations for structure: sub-issues, blocked-by and blocking links, labels, and milestones, not prose like "depends on #N".

## Supported Platforms

**Retired platforms are unsupported.** Do not add, restore, or retain support for a
retired platform in implementation code, dependencies, target selection, generated
projects, packaging, CI, tests, or documentation. Remove obsolete support paths
instead of maintaining compatibility shims or repairing retired targets. A failure
on a retired target is a reason to remove that target, not to extend its support.

Intel macOS (`x86_64-apple-darwin`) and Intel iOS simulators (`x86_64-apple-ios`)
are retired and unsupported. Apple support is ARM64-only: macOS
(`aarch64-apple-darwin`), iOS devices (`aarch64-apple-ios`), and iOS simulators
(`aarch64-apple-ios-sim`). Default target selection must exclude Intel Apple
targets, and explicit requests for them must fail with a clear unsupported-target
error. This does not remove x86 support from other supported operating systems.

## Framework Design Principles

These are constraints on every WaterUI feature, refactor, and review — not just the current task scope. They override convenience and they are not optional.

1. **Style is an attribute, not a separate component.** Toggle covers switch / checkbox; Picker covers menu / radio / wheel; List covers plain / inset-grouped / sidebar. Pick which visual via attribute (`.style(...)`, theme tokens, environment plugins, or backend platform default), never invent `CheckboxToggle` / `RadioPicker` / `GroupedList` parallel types. Semantic identity is fixed; visual presentation is a property of the surrounding context.

2. **Minimum native surface — compose in Rust before binding native.** Only widgets backed by a real platform primitive that cannot be expressed by composing existing primitives get a native port in the backends. `Form`, `Card`, `Badge`, `LabeledContent`, `GroupBox` are intentionally Rust-side composers that reuse `vstack` / `hstack` / `padding` / theme tokens and need no backend port. Adding a new native port requires evidence that no Rust-side composition produces the same result.

3. **"Native" means platform-coupled, not merely system-preinstalled.** A native implementation projects WaterUI semantics directly into the target platform's canonical object model, lifecycle, accessibility, input, graphics, or media pipeline. It may come from an OS framework or from an official extension package that is inseparable from that platform: Android View-based Material Components / MD3 count as native because they are coupled to Android's View, resource, accessibility, and graphics pipelines. A package is not native when it supplies a largely self-contained engine or runtime that owns the domain instead of bridging WaterUI into the platform, is meaningfully portable to other platforms, and substantially expands the application dependency closure. ExoPlayer / Media3, WaterKit, Zenwave, Hydrolysis, FFmpeg, GStreamer, Flutter, and React Native are not native implementations. Classify each layer independently: native controls, decoders, surfaces, or platform services do not make an application-owned playback or rendering engine native. `NativeView` is an internal backend-leaf marker and is not evidence that a realization satisfies this definition.

4. **Bridge native first, then provide the cross-platform self-drawn realization.** For each semantic component, first implement a native bridge on every platform that has a suitable native primitive. Also implement the shared self-drawn realization when the component needs a portable backend. When a platform has no suitable native primitive, go directly to the self-drawn realization; do not introduce a third-party parallel engine and call it native. The self-drawn realization is a deliberate backend, never a runtime fallback for a failed native path. Particle systems and QR codes have no suitable platform primitive and therefore start as self-drawn components. For WaterUI's video-player contract, Apple platforms bridge AVPlayer / AVKit as the only approved native player; every non-Apple platform uses the WaterKit / GPU-surface player, without ExoPlayer / Media3. Native controls, codecs, protected surfaces, media sessions, and output devices may still be used as platform sublayers around that self-drawn player.
   Map follows the same contract: Apple platforms bridge MapKit, while platforms
   without a suitable platform map primitive use WaterUI's MapLibre-style,
   Cherenkov / wgpu vector realization. A bundled portable map engine is not a
   native map. Native bridge failure is an error and must not silently switch
   realization at runtime.

5. **Native bridges must preserve tree shaking and proportional package size.** A bridge may make only the platform code and narrowly scoped support code required by the selected WaterUI features reachable in the final artifact. Do not hide a complete third-party framework or engine behind FFI, reflection, service registration, umbrella dependencies, or broad keep rules and label the result "native"; those boundaries can root the entire dependency graph and defeat R8, linker dead stripping, Cargo feature pruning, and equivalent size optimizations. Unused WaterUI features must remove their Rust code, platform code, resources, and transitive dependencies from the packaged application. Backend dependencies must be feature-granular, and any new runtime dependency requires measured before/after release-artifact size evidence on every affected packaging format. Wrapping the complete ExoPlayer / Media3 stack as an Android native video player is explicitly forbidden for both architectural ownership and package-size reasons.

6. **Cross-platform default appearance is the framework's job, not the view code's.** When a backend renders a primitive, it must read theme tokens (`Foreground` / `Background` / `Surface` / `SurfaceVariant` / `Border` / `Accent` / `MutedForeground` / `AccentForeground` / `AccentContainer` / `Tertiary` / `TertiaryContainer` / `SelectionContainer` / `SelectionForeground` / `Error` / `ErrorForeground`) instead of hard-coding `.label` / `.systemBackground` / `NSColor.windowBackgroundColor` / `UIColor.secondarySystemBackground`. View code calling `.foreground()`, `.background()`, `text("…")` etc. with no extra modifiers must produce platform-correct output. If view code has to reach into a backend to make defaults right, that is a backend bug — fix the backend, do not paper over it in user-facing code.

7. **Asymmetric primitives are documented, not faked.** When platform A has a primitive and platform B genuinely doesn't (e.g. SF Symbols on Apple vs no OS-supplied icon catalog on Android), the primitive is supported on A and **explicitly unsupported on B**. Do not bundle a Material font and pretend it is "system." For portable code, depend on a packaged icon-set crate (`waterui-icons-lucide`, `waterui-icons-material-icon`, `waterui-icons-fontawesome7`). Surfacing the asymmetry as documentation is the right answer; hiding it behind a fallback is not.

8. **Fine-grained reactivity is non-negotiable.** WaterUI uses precise per-`Binding` / `Computed` updates, not SwiftUI-style structural diff. APIs that would force a structural recompute on every state change (e.g. requiring rebuild of an entire subtree to update a single text value) are rejected. New API surfaces must accept signals (`impl IntoComputed<T>`, `impl Signal<Output = T>`, `Binding<T>`) rather than plain values when the underlying state is dynamic, so that callers never need `watch(...)` to drive them. How to author views against this model — `text!`, collections instead of `watch`, no `.snapshot()` in a body — is in the `waterui` skill.

9. **React-style local state slots are forbidden.** Do not introduce or depend on renderer-provided local state slot mechanisms such as `LocalStateScope`, `LocalStateStore`, `local_binding`, `with_local_binding_factory`, hook-like slot storage, body-position keys, or cursor-indexed state to preserve component-local state across body evaluation. WaterUI is not React: component identity and state must not be inferred from view body call order. Mutable UI state must be explicit `Binding` / `Computed` / `impl Signal` state owned at the correct semantic level and passed through the API or backend semantic object. If existing code needs renderer local slots to work, refactor that component/state model; do not fix a crash by teaching a backend to support the slot mechanism.

10. **Do not change WaterUI foundations without user approval.** Do not modify `core/`, foundational animation/reactivity/layout primitives, or shared backend contracts unless the user explicitly approves that foundation change in the current task. External references are evidence for values, semantics, and behavior; they are not permission to import another framework's abstraction model into WaterUI.

## Repository Boundaries and Distribution

These are the target architecture and acceptance criteria for repository changes. Existing in-tree implementations and backend submodules are migration state, not exceptions to the boundary. A layout description below is not evidence that a migration or release has completed.

- **WaterUI owns the foundations and the unified facade.** Core semantics, public backend contracts, and `waterui-graphics` APIs stay in the main repository. The `waterui` crate is a facade, like `crossbeam`: it re-exports independently maintained crates, including chart, rather than requiring every application to assemble the ecosystem manually. Use direct re-exports, not wrapper types or duplicate implementations. Keep heavyweight components optional through coherent Cargo features; disabling a feature must remove its dependency graph. Direct component imports and facade imports must expose the same types, including with dynamic linking.
- **Self-drawn components are independent versioned crates, not submodules.** SVG, canvas, chart, particle, barcode, and similar components own their implementations, tests, releases, and CI in their own repositories. They consume public `waterui-graphics` APIs and the core/layout/reactivity crates they actually need. Stable integration uses published crate versions. They must not depend on another repository's checkout layout or reach into its source tree for implementation or test resources.
- **`examples/` keeps only examples whose subject is a native widget or a core framework behavior.** The boundary is the same one the code follows: this repository keeps the native-widget core, self-drawn components live in their own repositories, and a self-drawn component's demonstration belongs to that component's repository too — the barcode demo lives in `water-rs/barcode`, the image demo in `water-rs/image`, not here. Move the complete example, assets, project configuration, and validation responsibility with the component; update workspace membership, CI paths, and links, and remove the main-repository copy once the standalone example works. Use the owning repository's component source and explicit compatible framework dependencies, without relying on the WaterUI checkout layout. `examples/filter` is the single documented exception: it stays because its subject is the framework contract of applying GPU filter effects to arbitrary views, including native widgets — a cross-layer interaction this repository is responsible for. And because that path is GPU-rendered, a backend's e2e suite must verify the filter example **visually, never by pixel-exact comparison**: GPU output is not pixel-stable across platforms, adapters, or execution paths.
- **Normal dependencies flow from the facade to components to foundations.** A component re-exported by `waterui` must not normally depend back on `waterui`. A backend that consumes the facade cannot also be re-exported by it until that reverse dependency is removed. Repository independence does not itself require a facade re-export of every backend.
- **The mainline backends and the first-party engine live in this repository.** Apple (`backends/apple`, AppKit and UIKit driven from Rust through objc2), Android (`backends/android`, Android Views driven from Rust through JNI) and Hydrolysis (`backends/hydrolysis`, the self-drawn renderer) are workspace members, and so is the engine Hydrolysis renders through: the `cherenkov` crates under `graphics/cherenkov/` and the `filtrate` family under `graphics/filtrate/`. An engine change and its consumers land in one pull request. Each keeps the native host code it needs, such as a Swift package or a Kotlin host library, beside its crate, and the framework and its backends change together in one pull request. Shared backend contracts stay in `backends/core`. The Kotlin Android runtime in water-rs/android-backend, pinned by `android-backend-revision`, keeps serving applications until the new Android backend replaces it. The experimental backends (GTK, WinUI, TUI) stay in their own repositories. Preserve each backend's rendering model; moving a backend is not permission to redesign shared contracts.
- **Style packages are independent repositories.** Hydrolysis is theme-independent. `hydrolysis-m3` (water-rs/hydrolysis-m3) and every other style package implement the public widget and theme contracts without depending on renderer internals, and they own their examples, assets, tests, CI and releases. A change to one is a pull request in its repository, never a crate in this tree.
- **Reserve submodules for inseparable core dependencies such as nami.** Being first-party, needing independent CI, or being distributed from a Git repository is not sufficient reason to add a submodule. Independently distributed components and backends are consumed through explicit compatible package versions or source revisions, not brought back as submodules. Existing non-core gitlinks must be migrated deliberately, not removed before their replacements work.
- **A split or a merge is complete only when both development lines are reconciled.** Compare the in-tree implementation with the standalone repository and the artifact actually consumed. Preserve unique fixes, tests, fixtures, documentation, and traceable source history from both sides. Account for local uncommitted work rather than silently overwriting or abandoning it. Verify both sides together before removing either copy. If the required public foundation APIs are unreleased, prepare compatible foundation and component releases through the release workflow; do not consume an older artifact that loses functionality or maintain a second implementation to hide the mismatch.

## CI Ownership and Framework Channels

- **Repository boundaries also define CI ownership.** Each external component or backend repository checks its own implementation and platform matrix, with triggers scoped to affected packages and paths. WaterUI checks its foundations, facade exports, its mainline backends and cross-repository integration. Its PR and push gate checks only the packages a diff affects together with their reverse dependencies, so a change confined to one backend does not run the others. A moved check must have a verified owner; no platform, feature combination, fixture, or correctness signal may disappear when code moves between repositories.
- **CI capacity: the `water-rs` organization is on GitHub Team, with 60 concurrent jobs in total and 20 of them macOS.** Every workflow is designed against that budget, and efficiency and parallelism are a standing concern on every CI touch, not a cleanup for later. Independent work fans out: matrix shards for the example sweeps, one concurrency group per dispatched run (a group shared by unrelated runs serializes them behind each other while runners sit idle), no job waiting on another it does not consume. Measure wall-clock from the Actions run itself, setup and cache restore included. Redundancy — the same dependency graph compiled twice in one run, a cache entry that restores nothing, a job whose output nobody reads, a cold build where a warm one was available — and any run far slower than the work it does are defects: warn the user immediately with the measurement rather than absorbing the cost silently.
- **The WaterUI dev-push gate is format plus compilation/lint checks, and must finish in under 10 minutes.** Full tests, doctests, examples, expensive platform/feature matrices, coverage, and other complex suites run nightly rather than on the dev-push critical path. Additional PR ABI/security checks remain explicit. Prioritize cache capacity for the frequent compilation gate. Prove the budget with actual Actions wall-clock measurements, including setup and cache overhead; setting a timeout or skipping correctness checks is not proof of success.
- **Build-time measurements cover only what users pay for.** Never measure the incremental build time of the WaterUI crates themselves, because nobody consumes that number. Measure two things:
  - the cold build time;
  - the incremental compile time and the `water preview` time of a real app that depends on WaterUI.
- **Framework channels are not Rust toolchain channels.** The CLI must support the following framework selections:

  | Channel | Source and guarantee |
  | --- | --- |
  | `dev` | The integration `dev` branch, resolved to an exact commit; compilation-checked, not test-certified. |
  | `nightly` | An immutable tagged revision whose complete required integration suite passed, with the tested dependency/backend combination. It is not a moving development branch or merely the latest commit of the day. |
  | `stable` | Formal crates.io releases with compatible native-backend versions; publishing remains owned by release-plz. Scaffold packages whose workspace requirement is a git pin are experimental: the stable `framework.json` omits them from `scaffold` and records them under `experimental-packages`. |

- **Promote tested snapshots, never moving inputs.** A nightly run captures one framework commit, its dependency lock, and exact backend/core references. Only after every required check succeeds may the promotion workflow create an immutable nightly tag and GitHub prerelease. Failed, cancelled, incomplete, or older runs must not replace or outrank a newer certified nightly. Keep the last successful nightly usable. Nightly promotion must not publish to crates.io or invoke stable release automation.
- **The CLI owns coherent version selection.** Resolve a channel on project creation or an explicit version change/update, then persist the channel and exact resolved versions. Normal build/run operations must not silently upgrade dependencies or mix a tested framework with independently advancing backend branches. Preserve the tested dependency lock for nightly rather than resolving newer compatible component releases. Stable uses published crates; local-checkout development remains explicit and must not override a requested channel implicitly. If no certified nightly exists, return an error rather than falling back to dev. Put resolution in the CLI library, not in terminal interaction wrappers.

- **Apple, Android, and Hydrolysis are the maintained mainline backends.** Their required integration checks, together with the CLI and the framework's own checks, define the release contract. Only these checks are prerequisites for a `dev` → `main` merge.
- **TUI, GTK, and WinUI are experimental backends, with no stability or feature-completeness guarantees.** Their nightlies are informational. Failures or missing functionality in these backends must not block mainline backend integration or releases. Record defects in the owning backend repository with the run ID and evidence, and proceed with the mainline work. Experimental backends may ship in the satellite wave when their manifests are ready, but never hold the framework or the other satellites back; do not dispatch control runs or debug them on the release's critical path.
- **Framework revisions own their minimum compatible CLI version.** Declare it in the root `Cargo.toml` under `[package.metadata.waterui].minimum-cli-version`, using a concrete SemVer version. Raise it when a framework change requires newer CLI behavior, not automatically for every CLI release. The library checks local source metadata, resolved `waterui` package metadata, and the requirement persisted with a channel before backend scaffolding or builds. Every published `framework.json` carries the same declaration. Rejection names the installed and required versions and gives an update command for the selected source; it never silently switches framework channels or upgrades the executable. Previously released CLIs without this check still need a one-time manual update.

## Layout Is Frozen

The layout system — the proposal protocol, `StretchAxis`, stack sizing and
allocation, every container in `waterui-layout`, and the leaf contracts backends
implement — is frozen as of 0.5.0. Its normative description is
[`docs/layout-spec.md`](docs/layout-spec.md). Do not change the semantics that
document describes, in `core/`, `components/foundation/layout/`, or a backend
bridge, for any reason: not to fix a parity budget, not to make an example
render, not because a reference framework does it differently. A difference
between the document and the code is a bug in the code; a pull request that
has to weaken a contract test or amend the document is rejected. The only path
to a semantic change is a major-version decision recorded by the maintainer.

Backend conformance fixes are permitted implementation work: repair a native
bridge so it implements the existing layout specification without changing
shared layout algorithms or leaf contracts. Such fixes do not require a
layout-decision label or separate foundation approval merely because they
affect placement. The label gate below applies to its listed shared-layout
paths, not to backend implementation files. A proposal to change the shared
algorithm, normative semantics, or contract still requires the maintainer's
decision.

`.github/workflows/layout-decision.yml` enforces this mechanically: a pull
request fails until it carries the `layout-decision` label whenever it can
change layout semantics — any edit to `docs/layout-spec.md`, a non-test Rust
file added or deleted under `components/foundation/layout/src/` or at
`core/src/ui/layout.rs`, or a change to such a file whose parsed syntax tree
differs after comments, doc comments, lint attributes (`must_use`, `expect`,
`allow`, `warn`, `deny`, `inline`, `doc`) and `#[cfg(test)]` items are removed;
`.github/scripts/layout_gate.py` makes that comparison. Lint-only,
documentation-only and test-only edits cannot change semantics, so they do
not need the label. Only the maintainer applies that label; an agent never
adds it, including to a pull request the maintainer approved in conversation.
It records his decision that the change is either a conformance fix (the code
now matches the document) or an approved amendment of the document.

A case the document does not decide is not a licence to pick a behaviour in
code. Every freeze break so far came from such a case found by a dogfood app;
record it as an issue with the code's current behaviour, the reference
framework's behaviour, and a recommended rule, and let the maintainer decide it.

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

Keep unrelated changes out of the change set.

- Keep top-level folders semantic and minimal. Do not add generic crate buckets (`crates/`), implementation-detail roots (`internal/`, `facade/`), or top-level folders whose only purpose is a single package manifest. Put crates under the existing domain folder (`components/`, `utils/`, `backends/`, etc. — icon sets live under `components/icon/`) or under `src/` when they describe the root `waterui` package itself. Crate families that share a non-`waterui` prefix belong under one family directory such as `utils/filtrate/`, not as repeated sibling folders like `filtrate-core` / `filtrate-derive`.

- **This repository is the core, the first-party engine, and the mainline backends, and nothing else.** What stays here is `waterui-core` and the reactive, layout and text foundations, `waterui-backend-core`, the Apple, Android and Hydrolysis backends, the `cherenkov` and `filtrate` engine family under `graphics/`, and the components whose realization is a native primitive on each platform. The `water` CLI is not part of it: it lives in water-rs/cli, pins the framework crates it links at one `water-rs/waterui` revision, and resolves everything else about the framework at run time through the channels below. Style packages (`hydrolysis-m3`) and every component that paints its own pixels (`waterui-svg`, `waterui-math`, `waterui-mermaid`, the crates listed in the next bullet) live in their own repositories under `water-rs` and iterate on their own schedule. Releases are the boundary's stable form: a split repository depends on the framework as published on crates.io, and this tree consumes the split repository from crates.io, so the `[workspace.dependencies]` requirement names a published version. Between releases the two sides move on git: an unreleased change in a split repository is consumed here through a `[patch.crates-io]` entry pinned to the exact commit on that repository's `dev` (`git = "https://github.com/water-rs/<repo>", rev = "<sha>"`), and an unreleased framework change is consumed there the same way against this repository's `dev`. Always a `rev`, never a moving `branch` and never a `path` patch, so the graph stays reproducible; the CLI's `dev` channel resolves the same way. A framework pull request never carries split-repository code, and neither repository blocks the other. **Never ask the user to publish, release, tag, or bump a version as a prerequisite for your work.** A release is the user's decision on the user's schedule; unless the user has stated a release plan in the current task, the only way to consume an unreleased change is the git pin above, and "waiting for a release" is never a state a task ends in. When a pull request needs a scaffold pin such as `waterui-gtk-version` to move to a version that does not exist yet, record that fact once in the pull request for whoever cuts the release, and finish the work. A new self-drawn component starts as its own repository, created from the `hydrolysis` template, and is never added to this tree.
- **Before adding a component crate, check whether the workspace already depends on one.** Several components were split into their own repositories and come back in from crates.io, so they are invisible when you search `components/` — `waterui-barcode` (QR and the other symbologies), `waterui-chart`, `waterui-map-gpu`, `waterui-canvas`, `waterui-particle`, `waterui-image`, `waterui-video-gpu`, `waterui-visualizer`, `waterui-math`, `shaderloom`, `nami`, `merman`. `waterui-mermaid` lives in water-rs/mermaid too, but its merman fork dependencies are git-only, so nothing here can consume it from crates.io — a git dependency is the only way to reach it. Read `[workspace.dependencies]` in the root `Cargo.toml`; each entry names a component that already exists, and a split component's example lives in that component's repository rather than in `examples/`. Extending one of those means a change in its own repository and a version bump here, never a second crate in this tree — and a QR-only type beside `Barcode` would be the parallel-type mistake Principle 1 rules out, since the symbology is an attribute of one semantic component.
- Prefer modern, cutting-edge tooling over legacy defaults when we choose or scaffold a toolchain — `bun` over `npm`/`yarn`, `uv` over `pip`/`pipenv`, and similarly for other categories. This is about the defaults we generate, never about overriding a project's declared toolchain: a lockfile or manifest in the user's project is the source of truth and is always respected.
- Do not drag unrelated files into the diff.
- Do not run workspace-wide formatters or refactors such as `cargo fmt --all`, bulk codemods, or broad search-replace when the task only targets a few files.
- Prefer file-scoped formatting and verification on the exact files you intentionally changed. For direct Rust file formatting, never run bare `rustfmt`; pass the workspace edition explicitly, for example `rustfmt --edition 2024 path/to/file.rs`, so rustfmt does not parse this Rust 2024 workspace as Rust 2015 and does not module-walk into unrelated files.
- Do not hardcode versions, repository URLs, package sources, filesystem paths, or other environment-derived constants just to ignore real complexity. If a value has a real source of truth, derive it from metadata, build inputs, repository structure, or runtime context instead of freezing a literal.
- **Tooling we ship must never recover semantics from user source code — no extracting configuration, paths, flags, attribute usage, or any other meaning by text search, regex, or AST parsing (`syn`, `ast-grep`, tree-sitter, or any other parser). No exceptions, for any reason.** Source text is not ground truth: it cannot see through macro expansion, `cfg` resolution, generics, or indirection, so scanning it produces wrong answers exactly where the language is doing its job. This constrains implementation behavior — e.g. the `water` CLI locating a frontend project — not an agent's own code reading. Values reach tooling through channels designed to carry them — filesystem layout, a manifest the tool owns, environment variables — or through **compiler-produced artifacts**: a macro may emit a symbol or a generated file, and tooling may enumerate the compiled artifact's symbol table or read files it generated, because the artifact is ground truth that has already resolved macros, cfgs, and generics. When no such channel exists, design and build the channel; do not scrape user code for one.
- **The canonical channel for a proc macro to convey metadata to the `water` CLI (or any tooling) is a metadata directory section in the compiled artifact.** The macro emits a `static` whose item name starts with `waterui_meta_`, marks it `#[used]`, parks it in a dedicated linker section — `.wmeta` (`__DATA,__wmeta` on Apple targets) — and makes its bytes a self-describing record: the item's name, NUL, the payload, NUL. The record format is the whole channel's contract, so it lives in `waterui-meta` (`utils/meta`) — a zero-dependency `no_std` crate holding the section name, the record builders (`dir_entry`/`dir_record`) and the section walker (`dir_records`); every side of the channel — emitters, decoders, the CLI — imports it directly. Put it behind `#[cfg(debug_assertions)]` — every such static, whatever consumes it, except a channel whose consumers need the record in every profile: a bundle mount record (`waterui_meta_bundle_*`) is read by `water build --release` too, so it omits both the debug gate and `#[used]`, relying on the crate's own rlib — archive members keep their bytes whether or not downstream code references them — while a shipped binary dead-strips it by design. `#[used]` keeps the item in the object file and rlib and defeats the linker's garbage collection; the reader then walks the section's records matching the `waterui_meta_` prefix on each record's name. The section exists because a symbol table cannot be trusted to: a linked MSVC image keeps no COFF symbol table at all (the PE header's symbol fields are deprecated and linkers write them as zero), so a static findable only by symbol name is unreachable there — the record's own name field is what identifies it, in objects, rlibs, and linked images alike. Never use `#[no_mangle]`/`#[export_name]`, which would put it in a cdylib's export table. Do not count on the final link to garbage-collect it: on Mach-O `#[used]` emits `no_dead_strip`, and a `#[used]` static in an app that shipped has been observed in the release executable. The debug gate is what makes the shipped binary carry zero bytes of it. This works because the CLI never reads metadata from the target build: it runs a dev-profile `cargo build --lib` of the user crate on the host and reads the resulting rlib (`build_host_rlib`), so packaging in release still sees every mount. Two consequences are part of the contract: the user crate must be a library (the CLI-generated FFI/bin wrapper is the leaf crate — a bin crate has no rlib to read), and a declaration behind a target-specific `cfg` (`#[cfg(target_os = "ios")] include_bundle!(..)`) is invisible to host discovery — declare mounts unconditionally. The artifact is ground truth that has already resolved macros, cfgs, and generics, so indirection that defeats source scanning cannot defeat this. Do not invent per-feature side channels — generated files, env hacks, source scraping; extend this one mechanism instead. Do not run external processes (bundlers, generators) inside a proc macro either: rust-analyzer re-expands macros on every edit, `cargo check`/`clippy` expand them too, and packaging builds several targets — expansion must stay a pure read; anything that runs a tool belongs to the CLI after the build.
- Do not add blind timing workarounds such as fixed sleeps, fixed-duration `RunLoop` waits, or arbitrary retry delays to "probably" wait for readiness. Wire the code to the real readiness/completion signal. If synchronous code must bridge to async readiness, keep driving the relevant event loop only until that concrete readiness condition completes.
- **`std::thread::sleep` is banned in tests. There is no exception for "just advancing an animation".** The animation clock advances when frames are pumped, not when wall-clock time passes, so a bare sleep freezes it: every deferred step is then applied at once on the next snapshot, and a capture meant to show a transition mid-flight silently shows its end state. The test still passes and the PNG still looks plausible, which is what makes this one dangerous. To sample a phase, pump: `OffscreenApp::pump_for(Duration)`. To wait for a condition, wait on the condition (`Query::wait_for_existence` and friends). The only sleep that belongs anywhere is the per-frame pacing *inside* a pump loop.
- Check `git status --short` before and after formatting or codegen steps. If unrelated files appear, stop and narrow the command instead of continuing with a polluted diff.
- Only use repo-wide formatting or sweeping rewrites when the user explicitly asks for them or the task genuinely requires touching the whole workspace.
- Follow **CI Ownership and Framework Channels** above. Preserve the full nightly suite and failure reporting while moving implementation-specific checks to their owning repositories. Do not treat a scheduled workflow as a certified distribution until its promotion and version-locking contracts are implemented.
- Workflow files under `.github/workflows/` may be changed WITHOUT asking when the change is a pure performance optimization that preserves coverage: cache keys and `save-if`/`cache-targets` tuning, job splitting or reordering, timeouts, runner sizing, moving a non-gating leg off the critical path onto a schedule, or adding a fast lane. A slow pipeline is a defect to fix, not a fact to endure. What still requires explicit authorization is any DEGRADATION: removing or skipping tests, dropping a platform or feature combination, loosening a lint gate, disabling a check, or trading correctness signal for speed. When in doubt about which side a change falls on, ask.
- GitHub Actions workflows should stay minimal and declarative. Do not put heavy release logic, repository analysis, packaging validation, or hand-rolled orchestration scripts into workflow YAML when a maintained community tool can own that behavior.
- Cross-Backend Regression is a CI pipeline concern, not user-facing README documentation. Keep references to it in CI/developer-maintainer context rather than public product docs.
- Prefer maintained community actions and purpose-built tools over custom shell/Python scripts in workflows. Release publishing should be delegated to `release-plz`; the only extra workflow glue this repository carries is the `framework.json` attachment on a facade release.
- A published CLI scaffolds projects against the `framework.json` that `.github/scripts/framework_manifest.py` certifies: the release job attaches it to each `v<version>` framework release, and `water channel nightly` reads the same shape from the last certified nightly prerelease. The manifest is derived from the framework's own root manifest, never from literals in the CLI; the script documents the derivation.
- Do not patch around repository-state problems by adding workflow preflight scripts or CI workarounds. Fix the source tree, manifests, submodules, or release configuration at the real source of truth.
- Do not add crate-level, file-level, or module-level `allow` attributes to skip lint failures during cleanup. Treat lint as code-quality feedback and fix the underlying code, API shape, docs, or type invariants instead.
- If a lint is a genuine false positive or conflicts with the intended architecture/readability, prefer a narrowly scoped item-level `allow`/`expect` with a concrete reason over contorting the code to satisfy the lint. WaterUI is a main-thread UI framework, so UI-local `spawn_local` futures that capture non-`Send` view state are a valid example. Do not use broad lint exceptions, and do not add exceptions without evidence.
- Use the `waterui` skill only when authoring WaterUI app/example code or checking public user-facing API usage.
- **Building an app or example is also a framework audit.** While authoring app/example code, every point where a WaterUI component or feature does not exist and its absence costs developer experience or user experience — a primitive you had to hand-roll, a modifier that should be built in, a gap you worked around — is a GitHub issue on this repository, filed under the same rules as any other finding. Do not silently absorb the friction into the example; the example's job is to expose it.
- The repo-local `.claude/skills/waterui/SKILL.md` is for WaterUI users. Update it only when a user-facing public authoring pattern, API usage rule, or app-level CLI usage changes.
- **Agent-facing surface: stateful is MCP, stateless is skill + CLI.** When a capability is aimed at a coding agent, pick its shape by whether the operation is a step in a conversation with a live process or a pure function of its inputs. A live session — `water mcp`, the offscreen app the agent drives turn by turn — is an MCP server, because the connection is the session: the agent host owns the process lifetime, and a crash is a visible tool error instead of a silent restart with reset state. A pure function — `water build`, `water test`, `water package` — is a CLI command documented in the skill, which keeps shell composition (pipes, `xargs`, CI) that MCP loses. A stateless operation whose output is an image the model itself has to look at — `water preview` — gets both: the CLI for scripts, and an MCP tool returning the image as image content, since the shell round trip costs the agent an extra turn just to read the file. Do not wrap a genuinely stateful session in stateless CLI verbs with session ids: the daemon dying and coming back fresh is invisible to the agent. Scripted, repeatable stateful interaction is neither surface — it is a `#[waterui::test]`.
- "Visual test" in this repository means the agent reads the generated image directly with its own vision capability. Heuristic image checks are forbidden, including changed-pixel counts, opaque-pixel thresholds, bbox approximations, dominant-color checks, brightness checks, non-uniform checks, and similar proxy code.
- Before writing any new image/gallery/snapshot export code, search for and reuse the existing `waterui-testing`, preview, showcase, GPU snapshot, or filter gallery infrastructure. Do not add ad-hoc gallery examples, scripts, or binaries unless the user explicitly asks to create or extend that infrastructure.
- For filter visual review images, the canonical reusable infrastructure is `cargo nextest run -p filtrate --lib -E 'test(gpu_export_filter_gallery_images)' --no-capture`, which exports PNG files to `/tmp/waterui_filter_gallery/`. Use this path to show filter outputs instead of creating a new gallery generator.
- Keep operational agent guidance in this file, not in product documentation or public change narratives.
- Keep `.claude/skills/waterui/SKILL.md` strictly user-facing. If information is primarily for agents or maintainers rather than app authors using WaterUI, it belongs in `AGENTS.md` or implementation docs, not in the user-facing skill.
- `.claude/skills/waterui/skill_snippets/` is the compile gate for `.claude/skills/waterui`: every rust fence in the skill is transcribed there (verbatim modulo rustfmt, with loudly-marked glue) and CI compiles it. When you change a skill code snippet, regenerate the matching module following the conventions in that crate's README. Its `#[waterui::test]` / `#[waterui::bench]` transcriptions sit behind the non-default `compile-gate-tests` feature: CI compiles them with `cargo check -p skill_snippets --all-targets --features compile-gate-tests`, and they must never be executed — they address elements that do not exist, by design.
- `waterui-testing` is based on the Hydrolysis accessibility tree, not native platform accessibility. Prefer `waterui-testing` for UI component coverage, and treat it as both an interaction test and an accessibility-correctness test.
- Every UI component is expected to produce a meaningful accessibility tree. If a component cannot be covered by `waterui-testing`, treat that as a bug to fix rather than a gap to paper over.
- Keep a component body's shape as simple and concrete as its semantics: do not wrap otherwise static content in a `Dynamic` because the body has a branch.
- Do not add `anyhow` as a direct dependency in any `Cargo.toml` in this workspace. The error type is re-exported as `waterui_core::Error`; reach for that re-export when implementing traits whose associated error is `anyhow::Error` (e.g. `Extractor`). `thiserror` and other error-construction utilities are unaffected.

<important>
    For rust: YOU CANNOT USE println, use tracing::debug!() instead for debug output.
    For swift: YOU CANNOT USE print(), use Logger instead for debug output. It uses `dev.waterui` as the log subsystem.
    For kotlin: YOU CANNOT USE println(), use Log.d() instead for debug output.

    Note that debug output will only appear if the CLI is run with --logs debug flag.

    For application creation, builds, previews, and execution, ALWAYS use the water CLI (water create, water build, water preview, water run, etc.). Do not bypass its application workflow with direct native-tool invocations.

    Standalone crate/backend-package verification uses the package's own toolchain: Cargo for Rust, SwiftPM for Swift packages, and Gradle for Android packages. This does not authorize hand-scaffolding an application or bypassing water for application deployment. If an application workflow requires direct adb/xcodebuild/other tool use because water lacks the capability, propose adding that capability to the CLI.

    Never hand-create or manually scaffold project/app structure. Always use `water create` (or existing generated project files) as the source of truth.
    For monorepo examples in local dev mode, `Water.toml` must explicitly set `waterui_path = "../.."` to force local backend usage and avoid remote backend resolution.
</important>

<important>
- Fail fast: when an unexpected case is encountered, crash early with a clear error message. No stubs, fallbacks, simplifications or patches, and no legacy code kept as a fallback: when a feature is deprecated, remove all of its code.
- Utilize rust's type system to enforce invariants at compile time rather than runtime checks.
- Prefer structs, traits, and generic abstractions over enums and type erasure when they express the intended model.
- Public traits expose the friendliest signature even when it is not object-safe (`-> impl Future`/`-> impl View`, generic methods, RPITIT). When dynamic dispatch is needed internally, do NOT degrade the public trait: add a private object-safe shim trait (`XxxImpl`) with a blanket `impl<T: Xxx> XxxImpl for T`, and store `Box<dyn XxxImpl>` behind a public wrapper type (`AnyXxx` / `ViewRenderer`-style). Type erasure is an implementation detail, never the user-facing API shape (see `core/src/ui/view_renderer.rs` for the canonical example).
- Backends dispatch on concrete configuration types, so every native/config type stores **erased** selection and item state: `Binding<Id>`, `Binding<Option<Id>>`, `Computed<Vec<PickerItem<Id>>>`. That erasure is deliberate and correct at that layer — do NOT report it as a design flaw, and do NOT try to make the configuration type generic. Keep it *below* the authoring layer instead: the public constructor stays generic over the app's own type and erases through `Mapping<T>` (`core/src/foundation/id.rs`), which assigns stable `Id`s and maps them back with `to_data`. Canonical pairs are `Picker::new<T>` → `PickerConfig`, `NavigationSplitView::new<T>` → `NavigationSplitLayout`, and `Tabs::new<T>` → `TabsLayout`. The bug to look for is the opposite one: a type that is simultaneously the authoring API and the raw view (`raw_view!` on the authoring struct) leaks `Id` into app code and forces callers to write `Id::try_from(1)` (no current component has this defect). Fix that by adding the generic constructor, never by changing the configuration type.
- Put shader to a separate file rather than embedding as string literal. Same for large text assets.
- Do not write duplicated code. If you find yourself copying and pasting code, consider refactoring it into a shared function or module.
- You are not allowed to revert or restore files or hide problems. If you find a bug, fix it properly rather than working around it.
- Never read back GPU render targets/textures to CPU memory in runtime render paths. This violates GPU-first architecture and causes severe performance degradation.
- Import third-party crates instead of writing your own implementation. Less code is better.
- `OffscreenRenderer` renders `GpuContent` and scenes without a surface. When developing any `GpuContent`-based component, you must use offscreen rendering for visual testing.
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
# Install the `water` CLI (required for `water run` to work). It lives in
# water-rs/cli; a change to it is a pull request there, not here. The installer
# downloads a prebuilt binary, so this is seconds rather than a full compile,
# and `water update` replaces it with the newest release afterwards.
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/water-rs/cli/releases/latest/download/waterui-cli-installer.sh | sh
# An unreleased CLI has no prebuilt artifact, so testing one does mean building
# it — from a checkout of water-rs/cli:
cargo install --path . --locked

# Build entire workspace
cargo build --workspace

# Run tests (nextest is the default runner; see "Testing Patterns")
cargo nextest run --workspace

# Run tests for specific crate
cargo nextest run -p waterui-core

# Run workspace doctests separately from nextest
cargo test --doc --workspace

# The mainline backends are workspace members
cargo build -p waterui-apple
cargo clippy -p waterui-android --target aarch64-linux-android
cargo build -p hydrolysis

# Run demo app (after creating a project)
water run --platform ios
water run --platform android
water run --platform linux --backend hydrolysis

# Create a project for quick experimentation
water create "My App"

# Preview a view function (renders to PNG without running full app)
water preview my_view --platform macos --path ./app --output preview.png
```

## Project structure

Every WaterUI project is entry-owning: WaterUI owns the program entry and the CLI generates and manages every backend project, so there is no Xcode or Gradle project in the project directory to edit. `water create` produces exactly this shape; there are no package types or modes to choose.

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
Rust view tree → backend contracts (waterui-backend-core)
  → Apple backend   → AppKit / UIKit, driven through objc2
  → Android backend → Android Views, driven through JNI
  → Hydrolysis      → Cherenkov (GPU, or CPU on microcontrollers)
  → experimental backends (GTK, WinUI, TUI) in their own repositories
```

### Layout of the repository

- `src/` — the `waterui` facade crate and its `prelude`
- `core/` — `waterui-core`: the `View` trait, `Environment`, `AnyView`, reactive primitives
- `components/` — component crates grouped by domain (`foundation/`, `visual/`, `multimedia/`, `platform/`, `devtools/`, `icon/`, …)
- `backends/` — `backends/core` (shared backend contracts) and the mainline backends
- `graphics/` — the engine family: `graphics/cherenkov/` (`cherenkov`, `cherenkov-record`, `cherenkov-shader`, `cherenkov-gpu`, `cherenkov-cpu`, `cherenkov-scene`, `cherenkov-oracle`, `cherenkov-bench`) and `graphics/filtrate/` (`filtrate`, `filtrate-core`, `filtrate-derive`)
- `utils/`, `macros/`, `testing/` — supporting crates, the proc macros and `waterui-testing`

The workspace members in the root `Cargo.toml` are the authoritative crate list.

Hydrolysis's own `AGENTS.md` carries its backend contract: the GPU and microcontroller design points, the frame model and its measurement rules. `graphics/cherenkov/AGENTS.md` carries the engine's standing contract — the oracle is the reference and the corpus stays bit-identical across non-pixel changes, the scene tree is generated by `scenes/tools/generate.py` and never committed or hand-edited, frames target 120fps (8.33 ms at p99) and CPU changes are accepted on deterministic Callgrind instruction counts — and `graphics/filtrate/AGENTS.md` carries the filter library's commands and test guidance. Both bind work under `graphics/`.

### CLI (water-rs/cli)

The `water` CLI orchestrates builds across platforms and lives in its own repository, https://github.com/water-rs/cli, with its own `AGENTS.md`, CI, nightly end-to-end suite and release cadence:

- `water create` - Scaffold new project
- `water run` - Build and deploy to device/simulator
- `water build --platform <platform>` - Build the project for the selected platform and backend
- `water package` - Package the production (release) build; `--debug` packages an unoptimized one
- `water clean` - Remove build artifacts
- `water doctor` - Check development environment
- `water devices` - List available devices and simulators

What this repository owns of that contract is the framework side: `[package.metadata.waterui]` in the root manifest (`minimum-cli-version`, backend coordinates, the Android API floor, `scaffold-packages`), the `framework.json` the release attaches, and the `waterui_meta_*` metadata symbols the macros emit for the CLI to read. A CLI defect is an issue in water-rs/cli; a fact the CLI cannot obtain from the framework is an issue here.

### Reactive System

Uses `nami` crate for fine-grained reactivity:

- `Binding<T>` - Mutable reactive state
- `Computed<T>` - Derived reactive values
- Views automatically update when reactive values change

### View Trait

```rust
pub trait View: 'static {
    fn body(self, env: &Environment) -> impl View;
}
```

## Key Development Notes

- Rust edition 2024; the supported toolchain floor lives in `rust-version` in the root manifest, not here
- Workspace lints enforce strict clippy rules including pedantic/nursery

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
  report, and prevent certified-version promotion when it fails.
  Keep the local check as well; moving webview to another repository must
  preserve the suite and its CI ownership. Passing Rust tests or real-engine
  integration tests does not replace this unit suite. It covers regressions
  that previously shipped behind green Rust checks: replies crossing as
  base64, a frozen `waterui` object breaking `state`/`watch`, and integers
  past 2^53 losing low bits in either direction.
- **The TypeScript runtime has a JavaScript unit suite that Cargo does not
  discover.** `components/platform/ts/src/js/` is injected into the JS engine
  as the `waterui` virtual module. If you touch it, run:

  ```bash
  bun test components/platform/ts/tests/js/
  ```

  Nightly runs it through `ts-js.yml` the same way as the web view bridge
  suite: a required check that blocks certified-version promotion.
- Use `tracing::debug!` and `water run --logs debug` for debugging runtime issues

### Error Handling

- Custom error enums use `thiserror` derive macro; the error type applications see is the `waterui_core::Error` re-export
