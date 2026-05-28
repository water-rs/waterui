# AGENTS.md

This file provides guidance to coding agents (Claude Code, Codex, and friends) when working with code in this repository. `CLAUDE.md` is a symlink to this file.

## Framework Design Principles

These are constraints on every WaterUI feature, refactor, and review — not just the current task scope. They override convenience and they are not optional.

1. **Style is an attribute, not a separate component.** Toggle covers switch / checkbox; Picker covers menu / radio / wheel; List covers plain / inset-grouped / sidebar. Pick which visual via attribute (`.style(...)`, theme tokens, environment plugins, or backend platform default), never invent `CheckboxToggle` / `RadioPicker` / `GroupedList` parallel types. Semantic identity is fixed; visual presentation is a property of the surrounding context.

2. **Minimum FFI surface — compose in Rust before binding native.** Only widgets backed by a real platform primitive that cannot be expressed by composing existing primitives belong on the FFI. `Form`, `Card`, `Badge`, `LabeledContent`, `GroupBox` are intentionally Rust-side composers that reuse `vstack` / `hstack` / `padding` / theme tokens and ship zero new C-ABI types. Adding a new `waterui_*_id()` requires evidence that no Rust-side composition produces the same result.

3. **Cross-platform default appearance is the framework's job, not the view code's.** When a backend renders a primitive, it must read theme tokens (`Foreground` / `Background` / `Surface` / `SurfaceVariant` / `Border` / `Accent` / `MutedForeground` / `AccentForeground`) instead of hard-coding `.label` / `.systemBackground` / `NSColor.windowBackgroundColor` / `UIColor.secondarySystemBackground`. View code calling `.foreground()`, `.background()`, `text("…")` etc. with no extra modifiers must produce platform-correct output. If view code has to reach into a backend to make defaults right, that is a backend bug — fix the backend, do not paper over it in user-facing code.

4. **Asymmetric primitives are documented, not faked.** When platform A has a primitive and platform B genuinely doesn't (e.g. SF Symbols on Apple vs no OS-supplied icon catalog on Android), the primitive is supported on A and **explicitly unsupported on B**. Do not bundle a Material font and pretend it is "system." For portable code, depend on a packaged icon-set crate (`waterui-icons-lucide`, `waterui-icons-material-icon`, `waterui-icons-fontawesome7`). Surfacing the asymmetry as documentation is the right answer; hiding it behind a fallback is not.

5. **Fine-grained reactivity is non-negotiable.** WaterUI uses precise per-`Binding` / `Computed` updates, not SwiftUI-style structural diff. APIs that would force a structural recompute on every state change (e.g. requiring rebuild of an entire subtree to update a single text value) are rejected. New API surfaces must accept signals (`impl IntoComputed<T>`, `impl Signal<Output = T>`, `Binding<T>`) rather than plain values when the underlying state is dynamic.

6. **Do not change WaterUI foundations without user approval.** Do not modify `core/`, foundational animation/reactivity/layout primitives, or shared backend contracts unless the user explicitly approves that foundation change in the current task. External references are evidence for values, semantics, and behavior; they are not permission to import another framework's abstraction model into WaterUI.

## Engagement Rules

DO NOT be over-engineer or write defensive code. If you encounter a problem, ask user for solution with your own idea, do not say "Let's have a simpler approach". You are expected to face the real problem and make code clean, reusable and elegant. Never take a workaround.

Keep the change set strictly scoped to the task.

- Keep top-level folders semantic and minimal. Do not add generic crate buckets (`crates/`), implementation-detail roots (`internal/`, `facade/`), or top-level folders whose only purpose is a single package manifest. Put crates under the existing domain folder (`components/`, `utils/`, `backends/`, `kit/`, `icon/`, etc.) or under `src/` when they describe the root `waterui` package itself. Crate families that share a non-`waterui` prefix belong under one family directory such as `utils/filtrate/`, not as repeated sibling folders like `filtrate-core` / `filtrate-derive`.

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
- Use `waterui-agent-workspace` for non-trivial repository implementation work. Creating that workspace copies the shared `target/` cache and is expected to be slow and quiet; do not treat long periods without output as suspicious by themselves. Use the `waterui` skill only when authoring WaterUI app/example code or checking public user-facing API usage.
- The repo-local `.claude/skills/waterui/SKILL.md` is for WaterUI users. Update it only when a user-facing public authoring pattern, API usage rule, or app-level CLI usage changes.
- "Visual test" in this repository means the agent reads the generated image directly with its own vision capability. Heuristic image checks are forbidden, including changed-pixel counts, opaque-pixel thresholds, bbox approximations, dominant-color checks, brightness checks, non-uniform checks, and similar proxy code.
- Before writing any new image/gallery/snapshot export code, search for and reuse the existing `waterui-testing`, preview, showcase, GPU snapshot, or filter gallery infrastructure. Do not add ad-hoc gallery examples, scripts, or binaries unless the user explicitly asks to create or extend that infrastructure.
- For filter visual review images, the canonical reusable infrastructure is `cargo test -p waterui-graphics --lib filter_view::tests::gpu_export_filter_gallery_images -- --nocapture`, which exports PNGs to `/tmp/waterui_filter_gallery/`. Use this path to show filter outputs instead of creating a new gallery generator.
- AI-agent meta belongs only in this file. Do not leak it into anything a human will read.
- Keep `.claude/skills/waterui/SKILL.md` strictly user-facing. If information is primarily for agents or maintainers rather than app authors using WaterUI, it belongs in `AGENTS.md` or implementation docs, not in the user-facing skill.
- `waterui-testing` is based on the Hydrolysis accessibility tree, not native platform accessibility. Prefer `waterui-testing` for UI component coverage, and treat it as both an interaction test and an accessibility-correctness test.
- Every UI component is expected to produce a meaningful accessibility tree. If a component cannot be covered by `waterui-testing`, treat that as a bug to fix rather than a gap to paper over.
- `GpuSurface::new(renderer)` owns one `GpuView` instance for that surface lifetime. `GpuView::setup()` is where persistent GPU resources for that renderer instance belong. Do not move renderer state into hidden shared caches just to survive `GpuSurface` teardown or parent rebuild.
- For text APIs, use `text()` for static text and `text!` for reactive formatting. Do not use `watch()` to build reactive text when `text!` or signal-taking APIs already express the dependency directly.
- Do not write `waterui::text!`. Always import the macro first, then use the short `text!` form.
- Do not inline absolute paths like `::waterui_core::views::ForEach` inside macro bodies or expanded code. Bring the names into scope with `use ...;` at the call site (or in the surrounding module) and reference them with bare identifiers — `ForEach`, `Collection`, `Identifiable`, `View`. The same applies to plain function/type usage: import first, use bare names. Long absolute paths add visual noise and break the look of declarative WaterUI code.
- Do not add `anyhow` as a direct dependency in any `Cargo.toml` in this workspace. The error type is re-exported as `waterui_core::Error`; reach for that re-export when implementing traits whose associated error is `anyhow::Error` (e.g. `Extractor`). `thiserror` and other error-construction utilities are unaffected.

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

# Run tests
cargo test

# Run tests for specific crate
cargo test -p waterui-core
cargo test -p waterui-cli

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
- **`hydrolysis/`** - Self-drawn renderer (Vello/tiny-skia) - experimental
- **`tui/`** - Terminal UI backend - WIP

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
    WaterUI uses precise fine-grained reactivity with Vue-like reconstruction semantics. A component's `.body` may be heavy and may perform one-time initialization for that component instance. After initialization, dynamic behavior is expected to be driven precisely through `Binding`, `Computed`, and other `impl Signal` inputs. If a component is recreated by control flow such as `when(...)`, `watch(...)`, or other parent-driven reconstruction, losing that component instance's local state is expected and correct because a new instance is being initialized. Do not preserve component-local state across rebuilds unless that state is explicitly owned at the correct reactive level.
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

- Rust edition 2024, minimum rustc 1.87
- Workspace lints enforce strict clippy rules including pedantic/nursery
- `backends/apple` and `backends/android` are git submodules inside the monorepo
- Start every new feature in its own git worktree (one worktree per feature branch)
- Worktree + submodule rules: after creating a worktree, run submodule init/update in that worktree and avoid switching submodule branches across worktrees; if you need parallel backend changes, use separate backend clones (or dedicated submodule checkouts per worktree) so submodule state does not collide between worktrees
- The FFI header `ffi/waterui.h` is checked into version control; CI verifies it's up-to-date; **never write C header by hand**
- When adding new components, update: Rust view → FFI exports → regenerate header → Swift component → Android component + JNI

### Testing Patterns

- Most tests use `#[cfg(test)] mod tests` pattern
- Run workspace tests: `cargo test`
- Run specific crate tests: `cargo test -p <crate-name>`
- No explicit CLI unit tests found; likely relies on integration testing
- Use `tracing::debug!` and `water run --logs debug` for debugging runtime issues

### Error Handling

- All command functions return `Result<(), eyre::Report>` for rich error context
- Custom error enums use `thiserror` derive macro
- Shell provides `success!()`, `error!()`, `warn!()`, `note!()` macros for user feedback
