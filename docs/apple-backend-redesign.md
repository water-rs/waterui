# Apple Backend Native Layout Redesign

## 1. Problem Statement
The current Apple backend is built almost entirely on SwiftUI (`backends/apple`, documented in `backends/apple/README.md`). SwiftUI is treated as a black box, so we have little control over diffing, scheduling, and layout. This creates several problems:

- **Opaque diffing & layout** – SwiftUI decides when views update, which makes it hard to guarantee performance or match the layout engine we use on other platforms (`components/layout/src/lib.rs`).
- **Divergent layout systems** – platforms like Android, Web, and Windows already rely on the Rust layout module. SwiftUI’s layout means Apple platforms behave differently.
- **Limited theming control** – system fonts/colors must be injected into the `Environment` (`core/src/resolve.rs`, `src/theme/mod.rs`, `utils/color/src/lib.rs`, `components/text/src/font.rs`). SwiftUI owns trait collection updates, so we can’t react to system theme/dynamic type changes consistently.
- **CLI ergonomics** – developers often re-run `water run …` commands with identical arguments; the CLI lacks a replay command and occasionally mishandles device selection (`water run ios --device=<device>`).

We want a design that keeps only the minimal SwiftUI boundary (Text/Image/etc.), moves everything else to native UIKit/AppKit/WatchKit implementations, and routes layout through the Rust engine while keeping system theming reactive.

## 2. Goals
1. **SwiftUI boundary minimization** – keep only `FixedContainer`, `Container`, `Text`, `Image`, and any unavoidable bridging types in SwiftUI. Everything else (buttons, lists, toggles, sliders, text fields, scroll views, etc.) moves to UIKit/AppKit/WatchKit renderers.
2. **Unified layout** – measure with native APIs, forward metrics into the Rust layout engine (`components/layout/src/lib.rs`), and apply the resulting frames back on platform views so every platform shares identical layout semantics.
3. **System theme injection** – during app startup, populate the WaterUI `Environment` with platform fonts/colors sourced from UIKit/AppKit trait collections. Theme values stay reactive: dark-mode flips, Dynamic Type changes, and high-contrast requests notify Rust so downstream views recompute.
4. **Latest Swift compatibility** – resolve actor-isolation issues (e.g., Swift 6 warnings/errors on `WaterUILayoutMeasurable`) by adopting `@MainActor`/`nonisolated` strategies and, where required, `@preconcurrency`.
5. **CLI usability** – add `water run again` to replay the last run configuration, and fix `water run ios --device=<device>` so it behaves like the interactive flow from `water run ios`.
6. **WidgetKit alignment** – keep SwiftUI wrappers available only where WidgetKit forces it; for other platforms (iOS/iPadOS/macOS/watchOS/visionOS) the SwiftUI boundary must be thin.

## 3. Non-Goals
- Re-implement WidgetKit renderers (WidgetKit still requires SwiftUI view trees).
- Change Rust layout algorithms; we only feed them different inputs.
- Replace the existing Environment/Resolver APIs; we integrate with them.

## 4. Architecture Overview

```
 Rust App ---------------------------+
  | components/layout, theme, env    |
  v                                  |
 C ABI (`waterui-ffi`)               |
  |                                  |
 Swift PlatformRuntime --------------+
  |                 |                |
  v                 v                v
 UIKit Hosts     AppKit Hosts     WatchKit Hosts
  | measure()      | measure()      | measure()
  +--------> Native measurement proposals
                  |
                  v
        Rust Layout (components/layout/src/lib.rs)
                  |
                  v
        Frame assignments -> apply to native views
```

### 4.1 Layer Responsibilities
- **Rust core** – produces `WuiAnyView` trees, theme/environment data, and consumes measurement callbacks to compute layouts.
- **CWaterUI** – ABI-safe structs for views, colors, fonts, layout proposals, and commands.
- **Swift PlatformRuntime** – retains the view tree, builds platform view hosts, calls native measurement, forwards metrics to Rust, applies layout frames, and keeps environment bindings in sync.
- **Platform Hosts** – UIKit/AppKit/WatchKit types implementing `WaterUILayoutMeasurable` and `PlatformViewDescriptor` to expose measurement + view metadata without using SwiftUI diffing.

### 4.2 SwiftUI Boundary
- Keep SwiftUI for:
  - `FixedContainer` and `Container` wrappers: they keep doing the minimal bridging inside `PlatformRenderer`.
  - `Text` / `Image`: we rely on SwiftUI for text shaping and symbol rendering until custom stacks exist.
  - WidgetKit adapters: still pure SwiftUI.
- Convert everything else to platform-native renderers; e.g., `UIKitToggleHost`, `UIKitTextFieldHost`, `UIKitSliderHost`, etc. Each host lives under `Sources/WaterUI/PlatformRuntime/UIKit` (and matching AppKit/WatchKit folders).
- Hosts conform to `WaterUILayoutMeasurable` via `@MainActor` extension to satisfy Swift 6 concurrency rules; measurement happens synchronously and we shield Rust from SwiftUI’s diff cycles.

### 4.3 Layout Pipeline
1. **Native measurement** – each host implements `func measure(in proposal: WuiProposalSize) -> CGSize`. The proposal is derived from Swift layout requests (e.g., `UIView.layoutSubviews` on UIKit). We call native APIs (`sizeThatFits`, `systemLayoutSizeFitting`, etc.) to gather intrinsic width/height, baseline, and layout priority.
2. **Measurement bridge** – the Swift runtime aggregates measurement data, constructs `WuiMeasuredNode` values, and calls back into Rust (new FFI: `wui_layout_measure_callback`).
3. **Rust layout** – `components/layout/src/lib.rs` computes final frames. Because it already powers other platforms, we reuse it for consistency; we only implement `PlatformMeasurements` to read Swift metrics.
4. **Frame application** – Rust returns a stable frame tree. Swift runtime iterates nodes, sets `UIView.frame` (or AppKit equivalents), and applies transforms/scroll bounds. When layout changes, we animate only if Rust flagged transitions (future work).
5. **Diff avoidance** – because measurement is separated from SwiftUI, we rely on explicit view descriptors and `PlatformViewGraph` nodes to diff + reuse hosts ourselves.

### 4.4 Environment & Theme Injection
- On app startup (e.g., `WaterDemoApp` before calling into Rust), we build a `SystemThemeSnapshot` from `UITraitCollection`, `NSFont`, and `NSColor`.
- Inject colors/fonts into the root `Environment` using a new `PlatformThemeProvider`. Values map to `Theme`, `ThemeColors`, and `ThemeTypography` tokens (`src/theme/mod.rs`).
- Observe system changes:
  - `traitCollectionDidChange` (UIKit) / `NSAppearance.didChangeNotification` (AppKit) → update color palette (light/dark, high-contrast, accent colors).
  - `UIContentSizeCategory.didChangeNotification` / `NSFontDescriptor` changes → update typography tokens (body/headline/… definitions from `components/text/src/font.rs`).
- Propagate updates by writing into `Environment` signals so dependent views recompute. Rust already keeps environment values reactive via `ThemeProvider`, so we call `env.set(theme_key, new_value)` to trigger watchers.
- WatchKit uses `WKInterfaceDevice.current().cachedDeviceTraitCollection`.
- This keeps fonts/colors consistent regardless of SwiftUI; only widgets rely on SwiftUI theming.

### 4.5 Platform Abstractions
- **UIKitRuntime**
  - Hosts for controls, list/grid containers, text fields, toggles, sliders, scroll views, gestures.
  - Implementations share base classes for event routing, measurement, accessibility, and theming hooks.
- **AppKitRuntime**
  - Mirrors UIKit layer but targets AppKit types (`NSView`, `NSTextField`, etc.).
  - Layout + measurement share the same FFI by conditionally compiling platform code.
- **WatchKitRuntime**
  - Provides wrappers for interface controllers; measurement approximations feed into Rust layout (watchOS layout is strict but we can map to `WuiProposalSize`).
- **WidgetKit**
  - Since WidgetKit must stay SwiftUI, we wrap the new infrastructure behind `PlatformRenderer`. WidgetKit code simply imports `WuiAnyView` and renders via SwiftUI components that already exist.

### 4.6 CLI Enhancements
- **`water run ios --device=<device>` fix** – the CLI will reuse the same selection code used in the interactive flow, ensuring devices resolved by name/UDID with helpful errors (`cli/src/backend/apple.rs`).
- **`water run again` command** – store the last successful `water run` invocation parameters in `.waterui/last-run.json` inside the project directory. Running `water run again`:
  - Loads the snapshot (platform, device, release/debug, hot reload, cache flags).
  - Rejects extra flags to avoid ambiguity; users must edit the JSON or run a new normal `water run`.
  - Works for iOS, macOS, Android, etc., and surfaces a friendly error if no snapshot exists.
- This supports rapid iteration during the rewrite without retyping long invocations.

## 5. Reactivity & Concurrency
- All UIKit/AppKit host classes adopt `@MainActor` to satisfy Swift 6; protocol requirements that must run off-main can be marked `nonisolated`.
- `WaterUILayoutMeasurable` becomes `@MainActor` since measurement touches `UIView/NSView`.
- Theme observers run on the main actor, publish updates through `AsyncStream`/`Combine` → Rust via FFI `wui_environment_update`.
- SwiftUI wrappers (Text/Image) stay `@MainActor`, but we sanitize pointers before bridging (already implemented in `PlatformRenderer` updates).
- For CLI concurrency, snapshot writes use atomic file replacement to avoid corruption.

## 6. Implementation Plan
1. **Scaffolding**
   - Define `PlatformViewGraph` nodes and diffing logic.
   - Introduce measurement FFI types shared with `components/layout/src/lib.rs`.
   - Mark measurement protocols `@MainActor`.
2. **UIKit Rewrite**
   - Port existing SwiftUI-based controls to UIKit hosts (toggle, text field, slider, button, progress, scroll, etc.).
   - Ensure hosts implement measurement hooks and event dispatch to Rust.
   - Add unit-style tests (XCTest) for measurement translation.
3. **Layout Bridge**
   - Implement measurement callbacks and frame application.
   - Validate round-trips with sample layouts (containers, stacks, scroll views) by comparing to current Rust-driven platforms.
4. **AppKit & WatchKit**
   - Mirror UIKit architecture, sharing logic via protocol extensions where possible.
5. **Theme Injection**
   - Create `PlatformThemeProvider` and watchers.
   - Map `UITraitCollection` + `UIContentSizeCategory` → `Theme` tokens.
   - Add integration tests toggling dark mode & dynamic type (using XCTest + trait overrides).
6. **CLI Enhancements**
   - Extend `RunArgs` parser for `again`, serialize snapshots, and add regression tests.
   - Fix device resolution for `--device` flag.
7. **Validation**
   - Run `water run ios --device=<device>` and `water run again` to ensure CLI and simulator start-up succeed.
   - Manual QA across iOS, macOS, watchOS simulators; verify layout parity with other platforms.

## 7. Risks & Mitigations
- **Layout regressions** – solve by building snapshot tests comparing Swift and Rust layout results; add logging behind a flag to inspect frame differences.
- **Trait-collection churn** – ensure environment updates are throttled (coalesce notifications) to avoid layout storms.
- **FFI complexity** – keep measurement structs ABI-stable; version them and gate mismatches with compile-time checks.
- **WidgetKit divergence** – create automated tests to ensure SwiftUI-specific code paths still work even as most of the runtime moves to UIKit/AppKit.

## 8. Open Questions
1. Do we need to support per-component opt-outs from the Rust layout engine (e.g., native stack layout for performance critical views)?
2. How should we expose platform theme overrides to user space—should there be a Rust API to query “system theme” separately from user-provided themes?
3. Can we share measurement code between UIKit and AppKit via a Swift macro or generics to avoid duplication?

This plan limits SwiftUI usage, adopts the Rust layout engine everywhere, keeps system theming reactive, fixes CLI ergonomics, and preserves WidgetKit compatibility while unlocking performance and debuggability across Apple platforms.
