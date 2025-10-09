# Android Backend Plan

This file tracks the standing plan for the Kotlin/Compose Android backend. Detailed design notes live in repository history; keep this document high-level so we can update status quickly.

## Milestones

1. **Project Scaffolding**
   - Finalise Gradle configuration for a Compose-enabled Android library.
   - Wire cargo/NDK build steps for `libwaterui_ffi` artefacts.
   - Define publishing coordinates for internal use.

2. **JNI Bridge Layer**
   - Implement native bindings that wrap the existing C ABI (initial glue lives in `runtime/NativeBindings.kt`).
   - Add helpers for array marshaling, string conversion, and pointer lifetime management.
   - Ensure safe threading model and deterministic disposal hooks.

3. **Runtime Core**
   - Complete `WuiEnvironment`, `WuiAnyView`, and `RenderRegistry` so Compose can traverse the Rust view tree.
   - Provide diagnostics/logging hooks for missing type IDs or failed coercions.
   - Implement cache invalidation strategies for registry overrides.

4. **Reactive Primitives**
   - Finish `WuiBinding`, `WuiComputed`, and watcher infrastructure using coroutines and `MutableState`.
   - Integrate animation metadata handling and lifecycle-aware disposal.

5. **Component Surface**
   - Flesh out Compose implementations for the initial component set (text, label, button, color, form widgets, layout container, scroll view, spacers, dynamic view).
   - Introduce preview providers and integration tests to exercise JNI interactions.

6. **Layout Bridge**
   - Build the three-pass layout handshake (propose → measure → place) in `layout/RustLayout.kt`.
   - Add performance tracing and fallbacks for unsupported Compose measurement modes.

7. **Testing & Tooling**
   - Automate instrumented smoke tests and Robolectric coverage for headless execution.
   - Integrate CI steps that cross-build the Rust artifacts and run Compose unit tests.

## Open Questions

- What net animation APIs should map from the FFI enum beyond `Default`/`None`?
- Can we reuse the Swift watcher helper patterns via `cbindgen`-generated structs, or do we need Android-specific wrappers?
- How should we package the Rust libraries (prefab, direct `.so`, or AAR with JNI libs)?

> Update this plan as pieces land so downstream contributors can see progress at a glance.
