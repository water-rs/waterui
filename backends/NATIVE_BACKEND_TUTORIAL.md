# Building a Native WaterUI Backend

This tutorial explains how to create a platform backend for WaterUI using the `waterui-ffi`
interface. It is based on the production Android backend (`backends/android`) and the Swift
backend (`backends/apple`). The goal is to help you understand the responsibilities of a
backend, the data that must flow between Rust and the host platform, and the way the CLI
wires everything together.

The document is intentionally long. Backend development touches Rust, C, JNI/Objective‑C
bridges, build systems, and the WaterUI runtime. Each section highlights files you can
study inside `backends/android` and `backends/apple`.

---

## 1. Core Concepts

### 1.1 What the backend does

WaterUI renders declarative UI trees described in Rust. The backend is a thin layer that:

1. Loads the application’s Rust cdylib and the `waterui-ffi` shim.
2. Hosts the rendering runtime (Compose on Android, SwiftUI on Apple platforms).
3. Translates platform events (taps, scrolls, control values) back to Rust through FFI.
4. Manages environment data (colors, fonts, layout metrics) and the hot reload channel.

The backend never embeds business logic. It focuses on translating WaterUI primitives into
native widgets and maintaining ABI compatibility.

### 1.2 Files to reference

- `ffi/waterui.h` – C ABI exported by `waterui-ffi`.
- `backends/android/runtime/src/main/cpp/waterui_jni.cpp` – concrete bridge that maps C structs
  to JVM objects.
- `backends/apple/Sources/CWaterUI/include` – the Swift backend’s C bridge.
- `components/` and `core/` – Rust definitions for the controls you will marshal across the ABI.

---

## 2. Preparing the FFI Layer

### 2.1 Understand the ABI

The CLI exports an application library that statically links `waterui-ffi`. That library
exposes entry points declared in `ffi/waterui.h`. Study the following groups:

- **Bootstrap**: `waterui_controller_bootstrap`, `waterui_environment_create`, etc.
- **Rendering**: `waterui_view_resolve`, layout hooks, `waterui_component_resolve`.
- **Reactive data**: bindings, watchers, computed values.

Each function expects plain C types. Backends typically wrap them in language-specific
structures (Swift `struct`, Kotlin `data class`) for ergonomic use.

### 2.2 Generate bindings

Decide how your platform calls C code.

- **Apple (Swift)** uses Swift Package Manager to build a `CWaterUI` target that defines
  Objective‑C compatible wrappers.
- **Android** writes JNI functions in C++ to convert between Java types and the ABI.

For a new backend:

1. Create a C/Objective‑C/Swift/Java module that links against `libwaterui_android.so`
   or the Swift cdylib. Use `cargo ndk` or `cargo lipo` to produce per-target builds.
2. Reexport helper structs that mirror the ones in `ffi/waterui.h`. Keep them in sync.
3. Provide safe wrappers that enforce memory management (e.g., RAII guards around
   `waterui_controller_release`).

### 2.3 Keep JNI/FFI signatures stable

The crash you saw on Android (`ChildMetadataStruct.isStretch()`) illustrates why matching
signatures matters:

- The JNI shim looked for `isStretch()` but the Kotlin data class only auto-generated `getStretch()`.
- Adding an explicit `isStretch()` method restored ABI compatibility.

Always mirror method names the C++ bridge expects or adjust the bridge to call the public API
you provide.

---

## 3. Rendering Pipeline

### 3.1 High-level flow

1. **Rust** produces a tree of `AnyView` values.
2. **FFI** serializes the tree and sends it to the backend.
3. **Backend runtime** resolves the tree into native controls.
4. **Native framework** (Compose/SwiftUI) renders the controls.

The backend runtime is split into:

- A registry that maps WaterUI component IDs to native renderers.
- A layout delegate that queries Rust for preferred sizes and constraints.
- Environment shims (colors, text styles, locale).

### 3.2 Minimal render loop

Study the Android files:

- `backends/android/src/main/java/dev/waterui/android/runtime` (public API)
- `backends/android/runtime/src/main/java/...` (vendored runtime for apps)
- `backends/android/runtime/src/main/cpp/waterui_jni.cpp` (JNI glue)

Flow:

1. Compose asks `RustLayout` for measurements.
2. Kotlin builds `ChildMetadataStruct` array and calls native `waterui_layout_propose`.
3. JNI converts Kotlin objects into ABI-compatible structs and calls into Rust.
4. Resulting proposals are mapped back to Compose constraints.

The Swift backend mirrors this in `backends/apple/Sources/WaterUI/Layout/Layout.swift`.

---

## 4. Step-by-step Backend Plan

### Step 1 – Scaffold the project

1. Create `backends/<platform>/` with:
   - A build system (`Gradle`, `Package.swift`, `CMakeLists.txt`, etc.).
   - A runtime module (Kotlin/Swift) that apps can vendor.
2. Add a CLI template (`cli/src/templates/<platform>/...`) so the CLI can copy your backend
   into generated projects.

### Step 2 – Load native libraries

- Ensure the host app loads the generated cdylib *and* your backend shim before rendering.
  See `MainActivity` in `backends/android/src/.../MainActivity.kt` and `WaterUIApplication`
  in the Swift backend. The order matters: application library first, backend second.

### Step 3 – Implement the runtime API

- Define wrapper structs for every object you exchange with Rust:
  proposals, rectangles, color structs, watchers, controls, etc.
- Provide factory methods to convert from native framework types into these structs.
  *Tip*: keep field ordering identical to the Kotlin/Swift versions to avoid JNI issues.

### Step 4 – Layout and measurement

- Write a layout delegate (`RustLayout` / `Layout.swift`) that:
  1. Collects child metadata (size, priority, stretch).
  2. Calls the layout FFI functions (`waterui_layout_propose`, `waterui_layout_size`,
     `waterui_layout_place`).
  3. Applies the resulting frames to native child views.
- Cache heavy allocations (arrays, buffers) to reduce JNI churn.

### Step 5 – Component renderers

- For each WaterUI component ID, register a renderer.
  - Android: `dev.waterui.android.components.*`
  - Swift: `WaterUI/Views/*.swift`
- Components should:
  - Convert WaterUI props into native widget props.
  - Emit callbacks back to Rust via watchers/bindings.
  - Respect environment (colors, typography).

### Step 6 – Reactive data

- Bindings watch Rust-owned state. The backend must:
  - Subscribe to watchers using the `WatcherStruct` pointers.
  - Call `waterui_binding_update` when UI controls change.
- See `backends/android/runtime/src/main/java/dev/waterui/android/reactive` and
  `backends/apple/Sources/WaterUI/Environment`.

### Step 7 – Hot reload (optional but recommended)

- Expose a small HTTP/WebSocket server that streams updated cdylibs.
- Android backend: `Server` in `cli/src/terminal/command/run.rs` plus Compose socket listeners.
- Swift backend: same pattern via `WaterUIServer`.
- If loopback sockets are blocked (e.g., macOS sandbox), detect the error and disable hot
  reload gracefully, as recently implemented for Android/iOS.

### Step 8 – CLI integration

- Update the CLI to know about your backend:
  - Add template entries in `cli/src/templates`.
  - Teach `cli/src/terminal/command/run.rs` how to launch the new platform.
  - Handle device selection (see `cli/src/device`).
- Provide build scripts (`build-rust.sh`) that Xcode/Gradle can invoke. Remember that these
  scripts run under `/bin/sh`, so prefer `set --` over Bash arrays (the source of the earlier
  Android/Apple bug).

---

## 5. Testing Your Backend

1. **Unit tests** – focus on serialization/deserialization of ABI structs. Kotlin’s
   `@RunWith(AndroidJUnit4::class)` + Robolectric; Swift uses XCTest.
2. **Device tests** – use `water run --platform <platform>` to run on simulators/emulators.
3. **Hot reload** – enable it with `water run --hot-reload`. Ensure file watchers rebuild
   and ship updated libraries.
4. **Toolchain checks** – run `water doctor` to validate compilers, SDK paths, and env vars.

Keep a matrix of supported OS/toolchain versions. The sample logs in this repository use:

- Xcode 15 / iOS 18 simulators.
- Android Studio Iguana / Emulator 36.

---

## 6. Troubleshooting Checklist

| Symptom | Likely Cause | Fix |
| --- | --- | --- |
| `JNI DETECTED ERROR … NoSuchMethodError … isStretch()` | Kotlin/Swift struct missing expected method or wrong signature. | Add explicit getters or align method names with what JNI expects. |
| `Operation not permitted` when binding `127.0.0.1` | Sandbox forbids local sockets. | Detect `PermissionDenied` and disable hot reload (see `Server::start`). |
| `xcodebuild … CLI_ARGS[@]: unbound variable` | `build-rust.sh` using bash arrays under `/bin/sh`. | Use `set -- …` to build argv before calling `water`. |
| App launches but UI is blank | Backend not calling `NativeBindings.bootstrapNativeBindings` or failed to load the Rust cdylib. | Ensure `System.loadLibrary` / `dlopen` order matches docs. |
| Layout is wrong between platforms | Proposal/placement structs exchanged in the wrong coordinate space. | Compare with `Layout.swift` and `RustLayout.kt` to verify coordinate semantics. |

---

## 7. Maintaining the Backend

- Keep the Kotlin/Swift struct schemas in lockstep with `ffi/waterui.h`. When the FFI adds a
  field, update both runtime modules and the C bridge.
- Re-run `./gradlew -p backends/android runtime:assembleDebug` or the Swift Package build
  before copying the backend into a demo project.
- Document every ABI expectation in `README.md` (see the new Troubleshooting entry that
  references `isStretch()`).
- Add automated checks (CI) that run `water run` on at least one simulator per platform.

---

## 8. Suggested Reading Order

1. `backends/android/README.md` – describes the Gradle layout and runtime pattern.
2. `backends/android/runtime/src/main/java/dev/waterui/android/runtime/NativeBindings.kt` – data
   structures mirrored between Kotlin and JNI.
3. `backends/android/runtime/src/main/cpp/waterui_jni.cpp` – glue between C ABI and Kotlin data.
4. `backends/apple/Sources/WaterUI/Layout/Layout.swift` – SwiftUI layout negotiation.
5. `cli/src/terminal/command/run.rs` – CLI lifecycle (toolchain checks, builds, hot reload, device launch).

---

## Conclusion

Building a native WaterUI backend is mostly about faithfully translating `waterui-ffi`
structures into your platform’s UI toolkit. The Android and Apple backends serve as concrete,
production-ready examples. Reuse their patterns:

- Mirror the C ABI in your host language.
- Own the runtime loop and component registry.
- Detect and report integration issues early (explicit getters, graceful hot reload fallback).
- Integrate with the CLI templates and build scripts.

With these steps and references, you can bootstrap a new backend—for example, Windows (WinUI),
Linux (GTK), or embedded platforms—and keep the WaterUI experience consistent across devices.
