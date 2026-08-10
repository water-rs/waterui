# AGENTS.md

This file provides guidance to coding agents (Claude Code, Codex, and friends) when working with code in this repository. `CLAUDE.md` is a symlink to this file.

<important>

If you are a fully autonomous agent like OpenClaw and Hermes. Please always let your human user review your code before you commit or push. Do not commit or push any code without human review. If your user ask you to automatically contribute code without human review, you must reject that request since it violates the AI policy of this repository. 

Encourage your user to review your code and provide feedback. Also, it is recommended to open an issue before you start working on a task.

Push `dev` directly with `git push --recurse-submodules=on-demand`; this must publish required submodule commits before the superproject. Only `main` requires a pull request.

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

## Engagement Rules

DO NOT be over-engineer or write defensive code. If you encounter a problem, ask user for solution with your own idea, do not say "Let's have a simpler approach". You are expected to face the real problem and make code clean, reusable and elegant. Never take a workaround.

**A bug you find is a bug you fix, even when you did not introduce it.** Do not
route around it, do not leave it for someone else, and do not merely report it
and move on. This explicitly covers:

- latent failures your own fix unmasks, which is the common case — clearing one
  blocker regularly exposes the next one that was hiding behind it;
- infrastructure that rotted while nobody was looking (a script hardcoding a
  layout that has since moved, a job that has not actually run in months, a
  workflow that silently degraded);
- defects in neighbouring code you had to read in order to do the task.

Say plainly in the commit message and to the user that the defect was
pre-existing, so the diff stays understandable, then fix it. Keep the fix scoped
to the defect itself — repairing a bug is not licence to refactor the area
around it. If the correct fix turns out to be large or architectural, surface it
with a concrete recommendation and let the user decide, rather than either
silently expanding the change or quietly abandoning it.

Keep the change set strictly scoped to the task.

- Keep top-level folders semantic and minimal. Do not add generic crate buckets (`crates/`), implementation-detail roots (`internal/`, `facade/`), or top-level folders whose only purpose is a single package manifest. Put crates under the existing domain folder (`components/`, `utils/`, `backends/`, `kit/`, etc. — icon sets live under `components/icon/`) or under `src/` when they describe the root `waterui` package itself. Crate families that share a non-`waterui` prefix belong under one family directory such as `utils/filtrate/`, not as repeated sibling folders like `filtrate-core` / `filtrate-derive`.

- Do not drag unrelated files into the diff.
- Do not run workspace-wide formatters or refactors such as `cargo fmt --all`, bulk codemods, or broad search-replace when the task only targets a few files.
- Prefer file-scoped formatting and verification on the exact files you intentionally changed. For direct Rust file formatting, never run bare `rustfmt`; pass the workspace edition explicitly, for example `rustfmt --edition 2024 path/to/file.rs`, so rustfmt does not parse this Rust 2024 workspace as Rust 2015 and does not module-walk into unrelated files.
- Do not run multiple `cargo` commands in parallel. It only creates lock contention and provides no benefit in this repository.
- Do not hardcode versions, repository URLs, package sources, filesystem paths, or other environment-derived constants just to ignore real complexity. If a value has a real source of truth, derive it from metadata, build inputs, repository structure, or runtime context instead of freezing a literal.
- Do not add blind timing workarounds such as fixed sleeps, fixed-duration `RunLoop` waits, or arbitrary retry delays to "probably" wait for readiness. Wire the code to the real readiness/completion signal. If synchronous code must bridge to async readiness, keep driving the relevant event loop only until that concrete readiness condition completes.
- Check `git status --short` before and after formatting or codegen steps. If unrelated files appear, stop and narrow the command instead of continuing with a polluted diff.
- Only use repo-wide formatting or sweeping rewrites when the user explicitly asks for them or the task genuinely requires touching the whole workspace.
- Workflow files under `.github/workflows/` are frozen unless the user explicitly authorizes workflow changes in the current task. Do not modify, add, delete, or rewrite workflow logic without that authorization. If a workflow change seems necessary, stop and ask first.
- GitHub Actions workflows should stay minimal and declarative. Do not put heavy release logic, repository analysis, packaging validation, or hand-rolled orchestration scripts into workflow YAML when a maintained community tool can own that behavior.
- Cross-Backend Regression is a CI pipeline concern, not user-facing README documentation. Keep references to it in CI/developer-maintainer context rather than public product docs.
- Prefer maintained community actions and purpose-built tools over custom shell/Python scripts in workflows. Release publishing should be delegated to `release-plz`; only the CLI binary prebuild/release-asset handoff is expected to require extra workflow glue.
- Do not patch around repository-state problems by adding workflow preflight scripts or CI workarounds. Fix the source tree, manifests, submodules, or release configuration at the real source of truth.
- Do not add crate-level, file-level, or module-level `allow` attributes to skip lint failures during cleanup. Treat lint as code-quality feedback and fix the underlying code, API shape, docs, or type invariants instead.
- If a lint is a genuine false positive or conflicts with the intended architecture/readability, prefer a narrowly scoped item-level `allow`/`expect` with a concrete reason over contorting the code to satisfy the lint. WaterUI is a main-thread UI framework, so UI-local `spawn_local` futures that capture non-`Send` view state are a valid example. Do not use broad lint exceptions, and do not add exceptions without evidence.
- MANDATORY for every agent (Claude Code, Codex, and any future agent): do non-trivial repository implementation work inside an **agent workspace**, never by editing the canonical checkout directly. "Non-trivial" = new features, multi-file refactors, backend/submodule changes, parallel agent tasks, or anything needing repeated build/test/rebase cycles. Read-only investigation and tiny localized edits may stay in the canonical checkout. The tooling is committed in-repo at `.claude/skills/waterui-agent-workspace/` (see its `SKILL.md`) so it travels with the repository and is identical for all agents — do not depend on any agent-private copy. The lifecycle, run from the repository root:
  - create: `.claude/skills/waterui-agent-workspace/scripts/create_workspace.sh <task-slug>` (clones the superproject + submodules locally, COW-copies `target/`, branches everything `agent/<slug>/<timestamp>`). Then `cd` into the printed workspace and do all edits/builds/tests there. Creating it copies the shared `target/` cache and is expected to be slow and quiet — long periods without output are not suspicious by themselves.
  - sync (when canonical advances): `.claude/skills/waterui-agent-workspace/scripts/sync_workspace.sh` from inside the workspace.
  - finish (merge back): `.claude/skills/waterui-agent-workspace/scripts/finish_workspace.sh` from inside the workspace — it takes the canonical integration lock, fast-forwards each submodule then the superproject onto their configured branches, and deletes the workspace. `finish_workspace` is a SCRIPT, not a built-in tool or agent command: invoke it, never hand-roll the fast-forward (a manual merge silently skips the submodule back-merge and the integration lock). All defaults are derived from git/`$HOME`; override only via `WATERUI_AGENT_SOURCE_REPO` / `WATERUI_AGENT_WORKSPACE_ROOT` / `WATERUI_AGENT_LOCK_ROOT` when a machine's layout differs.
- Use the `waterui` skill only when authoring WaterUI app/example code or checking public user-facing API usage; it is distinct from `waterui-agent-workspace` (workflow tooling for agents).
- The repo-local `.claude/skills/waterui/SKILL.md` is for WaterUI users. Update it only when a user-facing public authoring pattern, API usage rule, or app-level CLI usage changes.
- "Visual test" in this repository means the agent reads the generated image directly with its own vision capability. Heuristic image checks are forbidden, including changed-pixel counts, opaque-pixel thresholds, bbox approximations, dominant-color checks, brightness checks, non-uniform checks, and similar proxy code.
- Before writing any new image/gallery/snapshot export code, search for and reuse the existing `waterui-testing`, preview, showcase, GPU snapshot, or filter gallery infrastructure. Do not add ad-hoc gallery examples, scripts, or binaries unless the user explicitly asks to create or extend that infrastructure.
- For filter visual review images, the canonical reusable infrastructure is `cargo nextest run -p filtrate --lib -E 'test(gpu_export_filter_gallery_images)' --no-capture`, which exports PNG files to `/tmp/waterui_filter_gallery/`. Use this path to show filter outputs instead of creating a new gallery generator.
- AI-agent meta belongs only in this file. Do not leak it into anything a human will read.
- Keep `.claude/skills/waterui/SKILL.md` strictly user-facing. If information is primarily for agents or maintainers rather than app authors using WaterUI, it belongs in `AGENTS.md` or implementation docs, not in the user-facing skill.
- `waterui-testing` is based on the Hydrolysis accessibility tree, not native platform accessibility. Prefer `waterui-testing` for UI component coverage, and treat it as both an interaction test and an accessibility-correctness test.
- Every UI component is expected to produce a meaningful accessibility tree. If a component cannot be covered by `waterui-testing`, treat that as a bug to fix rather than a gap to paper over.
- `GpuSurface::new(renderer)` owns one `GpuView` instance for that surface lifetime. `GpuView::setup()` is where persistent GPU resources for that renderer instance belong. Do not move renderer state into hidden shared caches just to survive `GpuSurface` teardown or parent rebuild.
- For text APIs, use `text()` for static text and `text!` for reactive formatting. Do not use `watch()` to build reactive text when `text!` or signal-taking APIs already express the dependency directly.
- A **dynamic set of views** is a collection, not a `watch`: render it with `ForEach`/`List` over a reactive collection (`nami::collection::List`, `Identifiable` items) so membership changes diff by id. `watch(binding_of_vec, …)` rebuilds and re-dispatches the whole subtree on every change (and may escalate to a full-window structural rebuild) — that is the watch-abuse Principle #5 forbids. Authoring layer uses `ForEach`/`List`; the backend has `get_id`/`watch`/`get_view` to render it incrementally.
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
- Measurement caching is the `SubView`'s responsibility, never the `Layout`'s. The `Layout` trait deliberately has no cache — containers probe children freely with many proposals — so any caching (text shaping above all, which **must** cache) lives in the `SubView` implementation. Because layout measurement is designed to run in parallel across worker threads, a `SubView`'s cache must be thread-safe (a lock or lock-free map), not a `RefCell`. A `SubView` whose measurement must stay on the main thread confines its `!Send` state in `waterui_core::MainThreadBound` and returns `true` from `SubView::require_main_thread()`; parallelizable measures (text, media, shapes) return `false`. Do not add caching to `Layout`, and do not use a non-`Sync` cache in a `SubView`.

<important>
    For rust: YOU CANNOT USE println, use tracing::debug!() instead for debug output.
    For swift: YOU CANNOT USE print(), use Logger instead for debug output. It uses `dev.waterui` as the log subsystem.
    For kotlin: YOU CANNOT USE println(), use Log.d() instead for debug output.

    Note that debug output will only appear if the CLI is run with --logs debug flag.

    For any code that involves CLI commands, ALWAYS use the water CLI (water run, water build, etc.) to build and run the project.

    If you have to use adb/xcodebuild/other build tools directly, please propose adding a new command to the water CLI instead.

    Never hand-create or manually scaffold project/app structure. Always use `water create` (or existing generated project files) as the source of truth.
    For monorepo examples/playgrounds in local dev mode, `Water.toml` must explicitly set `waterui_path = "../.."` to force local backend usage and avoid remote backend resolution.
</important>

<important>
- Follow fast fail principle: if an unexpected case is encountered, crash early with a clear error message rather than fallback.
- Utilize rust's type system to enforce invariants at compile time rather than runtime checks.
- Use struct,trait and genetic abstractions rather than enum and type-erasure when possible.
- Public traits expose the friendliest signature even when it is not object-safe (`-> impl Future`/`-> impl View`, generic methods, RPITIT). When dynamic dispatch is needed internally, do NOT degrade the public trait: add a private object-safe shim trait (`XxxImpl`) with a blanket `impl<T: Xxx> XxxImpl for T`, and store `Box<dyn XxxImpl>` behind a public wrapper type (`AnyXxx` / `ViewRenderer`-style). Type erasure is an implementation detail, never the user-facing API shape (see `core/src/ui/view_renderer.rs` for the canonical example).
- Put shader to a separate file rather than embedding as string literal. Same for large text assets.
- Do not write duplicated code. If you find yourself copying and pasting code, consider refactoring it into a shared function or module.
- Always render on GPU rather than CPU
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
cargo build

# Run tests (nextest is the default runner; see "Testing Patterns")
cargo nextest run

# Run tests for specific crate
cargo nextest run -p waterui-core
cargo nextest run -p waterui-cli

# Doctests are the one thing nextest cannot run
cargo test --doc

# Generate FFI C header (after modifying ffi/ APIs), never write C header by hand
cargo run --bin generate_header --features cbindgen --manifest-path ffi/Cargo.toml

# Build Apple backend
cd backends/apple && swift build

# Build Android runtime
./gradlew -p backends/android runtime:assembleDebug

# Run demo app (after creating a project)
water run --platform ios
water run --platform android
water run --platform linux --backend hydrolysis

# Create a playground for quick experimentation
water create --playground --name my-playground

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

WaterUI is a cross-platform reactive UI framework that renders to native platform widgets (UIKit/AppKit on Apple, Android View on Android) rather than drawing its own pixels.

### Core Data Flow

```
Rust View Tree → FFI (C ABI) → Native Backend (Swift/Kotlin) → Platform UI
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

### Backends (`backends/`)

- **`apple/`** - Git submodule, Apple backend (Swift Package)
- **`android/`** - Git submodule, Android Views + JNI (Gradle project)
- **`hydrolysis/`** - Self-drawn GPU renderer (Vello on `wgpu`) - experimental. The high-end / game-engine renderer (see "Rendering backend philosophy" below)
- **`dew/`** - Self-drawn CPU renderer (`vello_cpu` sparse-strip) - experimental. The embedded / constrained-device renderer (see "Rendering backend philosophy" below)
- **`tui/`** - Terminal UI backend - WIP

#### Rendering backend philosophy: Hydrolysis vs Dew (self-drawn renderers)

WaterUI ships two self-drawn (non-native) renderers at deliberately opposite design points. They share `waterui-core`, reactivity, layout, and text, and diverge **only** in their render/flush strategy. Do not converge them, and do not port one's strategy onto the other — the divergence is the point. When touching either renderer, keep the change consistent with its half of this contract; a change that makes Hydrolysis frugal or Dew heavyweight is wrong by design.

**Hydrolysis — the game-engine renderer (high-end, future-facing).**
- GPU-first and GPU-required: rendering goes through Vello on `wgpu` with compute-shader support mandatory; there is no CPU rasterization path (`use_cpu: false`). Never add a CPU fallback, and never read GPU targets back to CPU.
- Full-scene redraw every frame, like a game engine. There is intentionally **no** dirty-rectangle / partial-region / damage tracking, and there must not be. Do not add "redraw only on change", region invalidation, or frame-skipping throttles — they contradict the design.
- Targets high-end modern devices and high frame rates (120fps and above). High-refresh must be requested **explicitly** per platform (opt into ProMotion / high-refresh display links), not left to incidental vsync. Do not introduce a hard frame cap.
- Designed to exploit modern hardware fully: modern GPU compute **and** multi-core CPU. Parallel scene building / rasterization across cores is part of the intended design; single-threaded execution is a gap to close, not the target. Do not assume or hard-wire single-threaded rendering.

**Dew — the embedded renderer (constrained, resource-frugal).**
- CPU-first; GPU is optional. The default and common path is pure-CPU rasterization (`vello_cpu` sparse-strip). It must run on MCU-class microcontrollers with no GPU and no full-resolution framebuffer.
- Dirty-area rendering is the core architecture, not an optional optimization: only changed regions are re-rasterized, sliced into horizontal bands, so peak pixel memory is one band — never a full frame. Do not introduce full-frame redraw into Dew.
- Modest, power-frugal frame rates: 30/60fps (the runtime ticks at ~16ms). Do not target 120fps here.
- Lean, feature-gated dependency graph: firmware builds strip `gpu`/`widgets`/`gestures` and other heavy deps (`default-features = false`). Dew is `std`-based via its embedded RTOS, not bare-metal `no_std`. Do not pull GPU / `wgpu` / heavyweight crates into Dew's firmware graph.

### CLI (`cli/`)

The `water` CLI orchestrates builds across platforms:

- `water create` - Scaffold new project (supports `--playground` for quick experiments)
- `water run` - Build and deploy to device/simulator
- `water build <target>` - Compile Rust library for platform (called by Xcode/Gradle)
- `water package` - Package built artifacts for distribution
- `water clean` - Remove build artifacts
- `water doctor` - Check development environment
- `water devices` - List available devices and simulators

**CLI Architecture Notes:**
- Entry point: `cli/src/terminal/main.rs` - Uses `clap` for parsing, `smol` async runtime
- Commands in `cli/src/terminal/commands/` - Each command is async and returns `Result<()>`
- Platform abstraction: `Platform` trait in `cli/src/platform.rs` implemented by `ApplePlatform` and `AndroidPlatform`
- Shell output: `cli/src/terminal/shell.rs` - Global singleton with human-readable (ANSI) or JSON modes

Note: `/terminal/*` (waterui-cli binary) only provide a friendly interface for CLI commands. All real logic should be implemented in the waterui-cli library part.

### FFI Contract

Native backends call into Rust via:

1. `waterui_init()` - Initialize runtime, returns Environment pointer
2. Theme installation (recommended):
   - `waterui_theme_install_color_scheme()` (light/dark)
   - `waterui_theme_install_color()` (slot-based colors)
   - `waterui_theme_install_font()` (slot-based fonts)
   - Legacy: `waterui_env_install_theme()` is deprecated
3. `waterui_main()` - Get root view tree
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
- `backends/apple` and `backends/android` are git submodules inside the monorepo
- Start every new feature in its own git worktree (one worktree per feature branch)
- Worktree + submodule rules: after creating a worktree, run submodule init/update in that worktree and avoid switching submodule branches across worktrees; if you need parallel backend changes, use separate backend clones (or dedicated submodule checkouts per worktree) so submodule state does not collide between worktrees
- The FFI header `ffi/waterui.h` is checked into version control; CI verifies it's up-to-date; **never write C header by hand**
- When adding new components, update: Rust view → FFI exports → regenerate header → Swift component → Android component + JNI

### Testing Patterns

- Most tests use `#[cfg(test)] mod tests` pattern
- **Use `cargo nextest run`, not `cargo test`.** This workspace is large and
  `cargo test`'s single-process-per-binary harness is painfully slow on it.
  Install once with `cargo install cargo-nextest --locked`.
  - Run workspace tests: `cargo nextest run`
  - Run a specific crate: `cargo nextest run -p <crate-name>`
  - Run one test: `cargo nextest run -p <crate-name> -E 'test(<name>)'`
  - Show output from passing tests: `--no-capture` (nextest's spelling of
    `-- --nocapture`; it forces serial execution, so scope it with `-E`)
- **`cargo test` remains correct in exactly two cases**, because nextest cannot
  run them:
  - **Doctests** — nextest has no doctest support at all. Run `cargo test --doc`.
  - Anything that depends on several `#[test]` functions sharing one process.
    nextest runs **each test in its own process**, so process-global state
    (`OnceLock`, `static`s, a registered "main thread", an installed global
    executor) is no longer shared between tests. That isolation is usually a
    feature — it turns order-dependent tests into deterministic ones — but a
    test written to rely on a sibling's initialization will start failing under
    nextest. Fix the shared-state assumption rather than reaching back for
    `cargo test`.
- No explicit CLI unit tests found; likely relies on integration testing
- Use `tracing::debug!` and `water run --logs debug` for debugging runtime issues

### Error Handling

- All command functions return `Result<(), eyre::Report>` for rich error context
- Custom error enums use `thiserror` derive macro
- Shell provides `success!()`, `error!()`, `warn!()`, `note!()` macros for user feedback
