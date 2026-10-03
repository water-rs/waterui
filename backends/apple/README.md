# WaterUI Apple Backend

[![License](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](#license)

The native Apple backend for the WaterUI framework, implemented in Rust on
objc2 (`waterui-apple`). It projects WaterUI semantics directly into
AppKit/UIKit — one `NSView`/`UIView` per realized view — with no FFI seam or
C header in between.

The Swift host is thin: `Sources/WaterUI/Embedding.swift` binds the
backend's exported entry points through `@_extern(c)` and is published
through the repository-root `Package.swift` as the `WaterUI` library.

## Scope

ARM64 Apple only: `aarch64-apple-darwin`, `aarch64-apple-ios`, and
`aarch64-apple-ios-sim`.

## Testing

```bash
cargo test -p waterui-apple                  # unit + doc
cargo nextest run -p waterui-apple \
    --features native-test-support           # native libtest-mimic suite
```

The same suite runs inside a booted iPhone simulator via the
`aarch64-apple-ios-sim` target runner in the root `.cargo/config.toml`;
see `.github/workflows/apple.yml` for the CI leg.

## History

Imported from the former `water-rs/apple-backend` repository; see
`PROVENANCE` for the exact source revision and the deltas applied.

## CI ownership

The backend's checks live in this repository:

- `.github/workflows/apple.yml` — the per-change gate: Swift lint and host
  builds, `cargo fmt`, and `cargo clippy` across the three Apple targets and
  the feature matrix (`map`, derived no-GPU set). This is the dev-push
  compile/lint gate.
- `.github/workflows/apple-e2e.yml` — the reusable suite invoked by the root
  `nightly.yml` through `workflow_call`: `host-native` (nextest, doctests,
  bench compile), packaging shards, screenshot captures, SwiftUI parity,
  the ios-device-tests native suite, and the release-metrics legs. One
  nightly run certifies the whole (framework, backend, lockfile, CLI)
  combination. Also dispatchable directly, and callable with
  `record_baselines`.
- `.github/workflows/apple-e2e-targeted.yml` — dispatch-only re-test of
  named examples without paying for the full suite.
- `.github/workflows/apple-bench.yml` — the informational criterion bench
  lane (weekly and on dispatch).
- `.github/workflows/apple-codeql.yml` — CodeQL Swift analysis.

Non-Apple `--workspace` commands `--exclude waterui-apple` rather than
compile AppKit/UIKit on Linux or Windows; the crate's own lanes own its
test suite end to end.
