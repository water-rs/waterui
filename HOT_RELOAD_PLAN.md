# Hot Reload Plan

## Goals

1. Enable runtime hot reload across all backends (Apple simulator, Android emulator/device, TUI, web).
2. Keep release/device builds free of hot reload code paths (no dlopen on app store binaries).

## Work Breakdown

### 1. Core Runtime (Rust)
- [ ] Ensure `waterui::Hotreload` + supporting code compile only when `waterui_enable_hot_reload` cfg is set.
- [ ] Provide a safe stub implementation when disabled (already done).

### 2. CLI Pipeline
- [ ] Provide per-platform hot reload enablement (Android ✅, TUI pending) in `platform_supports_native_hot_reload` and `run_platform`.
- [x] Start hot reload server for Android (with `adb reverse` or similar) and pass host/port info to the app.
- [ ] Support TUI platform runner that launches `waterui_tui` example, attaches `Hotreload` view, and points to CLI server.
- [ ] For Apple simulator/dev macOS: ensure Rust builds emit `Hotreload::new(...)` (already in template) and that `WATERUI_ENABLE_HOT_RELOAD` is only set for simulator/debug artifacts so device/release binaries never include dlopen logic.
- [ ] Manage incremental rebuilds per platform (native cdylib vs wasm).
- [ ] Document that physical iOS devices cannot use hot reload due to code signing.

### 3. Apple Runtime
- [ ] Leverage the existing Rust `Hotreload` view so Swift remains a dumb host: verify simulator builds still receive dynamic updates purely through FFI.
- [ ] Keep fallback path (no hot reload) for release/device builds by never defining `waterui_enable_hot_reload` on those slices.

### 4. Android Runtime
- [x] Reuse new `configureHotReloadEndpoint` JNI hook to set host/port from Compose.
- [x] Provide a minimal hook (`configureHotReloadDirectory`) so the Rust runtime knows where to persist downloaded `.so` files (e.g., `codeCacheDir`). All networking/dlopen logic remains in Rust.
- [ ] Ensure template + runtime docs explain the sandbox-friendly directory requirement.

### 5. TUI Backend
- [ ] Introduce CLI platform option (e.g., `--platform tui`) with hot reload support (server + watcher already reuse native path).
- [ ] Create a sample entry that wraps root view with `Hotreload` and listens to CLI in the same filesystem.

### 6. Web Backend
- [ ] Current behavior reloads the entire page via WebSocket; acceptable short-term. (Future: fine-grained module reload.)

### 7. Docs & Templates
- [ ] Update `cli/src/templates/lib.rs.tpl` (done) and platform templates to show how hot reload is enabled.
- [ ] Update `IMPLEMENTATION_STATUS.md` files for each backend to reflect hot reload progress.
- [ ] Add troubleshooting section for hot reload (port conflicts, `adb reverse`, iOS limitations).

## Immediate Next Steps

1. Extend CLI hot reload server support to Android and TUI; ensure `waterui_enable_hot_reload` env var is set appropriately per target triple.
2. Implement Android hot reload client (OkHttp + `System.load`) and surface state updates in Compose runtime.
3. Add Swift `HotReloadController` for simulator builds.
4. Wire CLI to pass host/port via env vars / intent extras / `SIMCTL_CHILD_*` for each platform.
