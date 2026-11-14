# Apple backend AGENTS.md

`waterui-apple` is the native Apple backend: Rust on objc2 projecting WaterUI semantics directly into AppKit and UIKit, one `NSView`/`UIView` per realized view, with no FFI seam or C header in between. The rules in the root [`AGENTS.md`](../../AGENTS.md) apply to every file here too; [`README.md`](README.md) carries testing commands and the CI lanes in full.

## Layout

- `src/` — the backend: component ports (`src/components/`), layout bridging, dispatch, windows and scenes, preview, capture
- `cocoa-ui/` — the safe Rust API over AppKit and UIKit the backend builds on
- `Sources/WaterUI/` — the thin Swift host: `Embedding.swift` binds the backend's exported entry points through `@_extern(c)`, published through the repository-root `Package.swift` as the `WaterUI` library
- `Tests/` — the native suites (`native`, `native_app`, `preview`; `harness = false`, behind `native-test`), the iOS test host and the SwiftUI reference hosts and baselines
- `benches/` — `hot_paths`, CPU-side benchmarks of mount, update, layout, reconciliation and table-cell reuse; informational, never a gate

## Rules

- The native suites own the real process main thread: run them with `cargo nextest run -p waterui-apple --features native-test`; on `aarch64-apple-ios-sim` the target runner in the root `.cargo/config.toml` runs them inside a booted simulator.
- Non-Apple `--workspace` commands `--exclude waterui-apple --exclude cocoa-ui` instead of compiling AppKit/UIKit on Linux or Windows; this crate's own lanes (`apple.yml`, `apple-e2e.yml`, `apple-e2e-targeted.yml`, `apple-bench.yml`) own its suite end to end.
