---
name: waterui
description: Build cross-platform apps with WaterUI. Use when writing views, handling state, styling UI, or debugging WaterUI Rust code. Covers reactive bindings, layout, components, and the water CLI.
---

# WaterUI App Development

Build views with reactive state. When unsure, use Explore agent to search `examples/*/src/lib.rs`.

## CRITICAL: Runtime And Testing Semantics

- WaterUI is fine-grained reactive with reconstruction semantics. If parent-driven control flow rebuilds a component instance, that instance's local state resetting is expected and correct.
- Do not "fix" rebuild-driven resets by caching hidden state across rebuilds. If state must survive, lift it into explicit reactive ownership at the right level.
- GitHub Actions workflows should stay minimal and declarative. Prefer maintained community actions and purpose-built tools over custom shell/Python scripts; release publishing belongs to `release-plz`, with only the CLI binary prebuild/release-asset handoff needing extra workflow glue.
- Cross-Backend Regression is a CI pipeline concern, not user-facing README documentation. Keep references to it in CI/developer-maintainer context rather than public product docs.
- Do not add workflow preflight scripts or CI workarounds to hide repository-state problems. Fix the source tree, manifests, submodules, or release configuration at the real source of truth.
- Lints are quality feedback, not obstacles. Do not add crate-level, file-level, or module-level `allow` attributes during cleanup; fix the code, docs, API shape, or type invariant instead. When a lint is a genuine false positive or conflicts with intended architecture/readability, use the narrowest item-level `allow`/`expect` with a concrete reason instead of distorting the code; WaterUI's main-thread `spawn_local` UI futures may intentionally capture non-`Send` view state.
- `waterui-testing` uses the Hydrolysis accessibility tree, not native platform accessibility. Use it to validate both interaction logic and accessibility correctness.
- Every UI component is expected to expose a meaningful accessibility tree. If Hydrolysis coverage is missing, fix the component or renderer rather than falling back to weak tests.
- "Visual test" means the agent reads the generated image directly with its own vision capability. Heuristic image checks are forbidden: no pixel counters, threshold diffs, non-uniform checks, dominant-color checks, bbox approximations, or similar proxy code.
- `GpuSurface::new(renderer)` owns a single `GpuView` instance for that surface lifetime. `GpuView::setup()` is the place for persistent GPU resources tied to that renderer instance. Do not move renderer state into shared caches to survive teardown or parent rebuild.
- Hydrolysis production surfaces must continue to reject software/noop adapters unless explicitly forced for diagnostics. Renderer unit tests that need a real `wgpu::Device` but run in CI may use the `#[cfg(test)]` offscreen test constructor; keep that permission test-only and continue requesting Vello-compatible default wgpu limits for compute-capable adapters.
- `waterui-testing` is a first-class Hydrolysis test host. It may enable Hydrolysis's `testing` feature and use `new_for_tests` constructors so CI accessibility/coverage tests can run on compute-capable llvmpipe, but production constructors such as `OffscreenWindow::new` and `HeadlessRuntime::new` must keep rejecting software/noop adapters by default.
- `waterui_graphics` shared GPU context is also a Hydrolysis rendering entry when views use `.hydrolysis().render_offscreen(...)`. Keep its device-limit selection aligned with Hydrolysis offscreen surfaces: compute-capable downlevel adapters such as CI llvmpipe still need `wgpu::Limits::default()` rather than `downlevel_defaults()` for Vello-backed rendering.
- Shared-context offscreen rendering must hold the shared offscreen-operation lock through renderer setup, rendering, readback, and explicit renderer/texture teardown. Linux CI llvmpipe uses the GLES backend, whose adapter context can panic if parallel tests create or destroy Vello compute pipelines on the shared device with overlapping lifetimes.
- Managed playground build-cache GC is an out-of-band maintenance path exposed as `water gc build-cache`. `water preview` and `water run` may trigger that command in a detached subprocess, but they must never scan `~/.water/build_cache` on the hot path.
- Preview/inspector dev mode is only valid against a clean local `waterui_path` git worktree. Dirty WaterUI worktrees must fail fast; release mode should resolve WaterUI from registry metadata instead of forcing a local path checkout.
- Preview requires the root crate to expose `[features] dev = ["waterui/dynamic_linking"]`. The generated preview wrapper (`managed_backends/preview_ffi`) must always depend on the app crate with `features = ["dev"]`, and it should emit only a single preview artifact (`dylib`) rather than extra Rust-library outputs. macOS, iOS Simulator, physical iOS, and Android preview builds all use this dylib path; do not special-case iOS or Android back to a static `cdylib` preview build.
- Preview TCP handshake must validate the support app runtime platform as well as the WaterUI runtime fingerprint. iOS/Android preview sessions must not reuse a macOS support app just because the runtime fingerprint matches; that causes incompatible-platform `dlopen` failures for dylib preview payloads.
- Preview capture must not race normal display renders for the same GPU surface. When switching a surface into external rendering or waiting for first-paint readiness, drain any already in-flight render through the shared render queue barrier before acquiring the surface again; do not use sleeps or static preview fallbacks.
- Apple `ios-simulator` packaging must pass `ONLY_ACTIVE_ARCH=YES` through `xcodebuild` build settings. `ONLY_ACTIVE_ARCHITECTURE=YES` is ignored by Xcode, which silently falls back to building/linking multiple simulator architectures and can break helper static-library linkage when WaterUI only produced the active-arch artifact.
- Prefer `#[waterui::test(view_fn)]` when a test only needs the default `UiTest::new().mount(view_fn)` setup. Keep explicit `UiTest` construction only when the test genuinely requires a custom viewport or environment.
- When testing layout containers with `waterui-testing`, prefer inherently semantic child views such as `text()`, buttons, or other labeled controls, then assert bounds relationships from the Hydrolysis tree. Do not pad test counts with decorative color blocks plus synthetic metadata if a semantic child expresses the behavior more directly.
- Chart semantic readout fixtures should preserve the chart surface geometry used by hit-test helpers. If long focused/selected readouts need more vertical budget in a fixed viewport, tune the shell chrome such as inter-item spacing rather than shrinking the chart and invalidating normalized interaction coordinates.
- For static components with simple conditional branches, avoid wrapping the whole body in `#[view_builder]` if that would introduce an unnecessary `Dynamic`. `waterui_chart::Tooltip` is a concrete example: explicit `AnyView` branching avoids a Hydrolysis mount-time recursion path that appeared with the generated dynamic wrapper.
- Playground root crates are plain Rust `lib` crates. Do not make playground examples choose final artifact types such as `staticlib`, `cdylib`, or preview dylib output themselves. App FFI artifacts and preview dylibs must be produced by generated wrapper crates such as `managed_backends/ffi`, with the playground root crate only supplying normal Rust APIs like `app(...)` and `#[preview]` exports.
- `Project::open(OpenMode::Full)` must initialize the managed Android backend for playground projects before scaffolding the FFI companion. If Android packaging reports a missing backend on an already-open playground, treat that as a CLI bug.
- Android build/package preflight must validate only the Rust targets required by the requested Android ABIs. Requiring every Android target when the command asked for a single ABI is incorrect.
- Android Gradle `package` and `clean` invocations must export detected `ANDROID_HOME` and `ANDROID_SDK_ROOT`. Included builds and composite local backends do not reliably rediscover the SDK on their own.
- Android `doctor --fix` must derive the NDK package version from `backends/android/runtime/build.gradle.kts` `ndkVersion`, not by guessing the newest installed or newest published NDK.
- Android `doctor --fix` on Linux, including native ARM Linux containers, must bootstrap Android SDK command-line tools into the configured/default SDK root itself. Do not require manual SDK/NDK downloads or pre-block ARM Linux before `sdkmanager` has actually been tried.
- Android NDK resolution should prefer the runtime-declared `ndkVersion` directory under `ANDROID_SDK_ROOT/ndk` over any lexicographically newer leftover installation. A newer stale directory must not override the runtime source of truth.
- When `doctor --fix` targets an Android NDK version and that directory exists but is incomplete (for example missing `toolchains/llvm/prebuilt`), the CLI must delete that damaged directory first and then re-run `sdkmanager --install` for the declared version. Retrying install on top of a broken directory is not sufficient.
- Android Kotlin compiler compatibility must come from `backends/android/settings.gradle.kts` `org.jetbrains.kotlin.android` version, but the CLI must embed that version at build time so managed-build-cache packaging does not depend on the current working directory. `Kotlin::detect_path()` and `doctor --fix` must both use that embedded version source.
- `kotlinc -version` parsing must ignore unrelated JVM warning lines and only extract the version token from the `kotlinc...` output line. Parsing the first numeric token in the combined stdout/stderr stream is incorrect and can falsely accept an old compiler because of warnings like `JDK 13`.
- Android managed-backend templates are embedded at CLI compile time. After changing files under `cli/src/templates/android`, rebuild the CLI before validating generated playground backends; otherwise the generated Gradle files can still reflect stale template contents.
- Keep the Android template/backend version matrix aligned with the runtime dependency graph. The current matrix requires `compileSdk = 36`, `targetSdk = 36`, and AGP `8.9.1`; drifting below that breaks fresh playground packaging.
- Android app templates must enable core library desugaring whenever the runtime backend exposes a desugaring requirement. Keep the app template's `coreLibraryDesugaring` dependency aligned with `backends/android/runtime/build.gradle.kts`.
- Android asset staging must preserve a valid default launcher foreground resource when the project has no custom app icon asset. Replacing `ic_launcher_foreground.xml` with generated raster resources is only correct when a custom icon is actually staged; otherwise the staging step must restore the default vector resource and remove stale rasterized launcher artifacts.
- GitHub Actions jobs that run Cargo against the superproject must check out submodules recursively. Missing `vendor/nami` or backend submodules in CI is a workflow bug, not a Rust dependency bug.
- Linux GitHub Actions jobs that compile the workspace with all features should not depend on `nasm` for WaterKit AV1 fallback. Keep `rav1e` default features disabled and enable only the required Rust-side features such as `threading`.
- Linux Wayland-facing dependencies must not require compile-time `wayland-client.pc` unless the workflow explicitly installs Wayland development packages. Prefer portal/dlopen-backed dependency features so CI and downstream consumers can compile without native development headers while preserving runtime Wayland support.
- Linux GitHub Actions jobs that compile real audio/video/font/graphics backends must install native development packages for the enabled backends, including `libasound2-dev` for ALSA audio, `libva-dev` for VA-API codecs, `libfontconfig1-dev` for Fontconfig text/font discovery, `libgbm-dev` for GBM GPU surface linking, `libpango1.0-dev` for Pango text layout pkg-config discovery, `libgdk-pixbuf-2.0-dev` for GDK Pixbuf image loading, and `libgtk-4-dev` for the GTK backend. This includes `cargo hack --each-feature`, because single-feature checks can still enable those backend crates. Do not remove backend capabilities just to avoid the packages.
- On newer Cargo, `cargo llvm-cov` with the default `target/llvm-cov-target` scratch directory must start from a directory that contains a valid `CACHEDIR.TAG`, otherwise Cargo aborts the clean step before coverage begins.
- Docker Linux containers do not automatically inherit the host macOS `sing-box` TUN path. For Android verification inside Docker/OrbStack, treat `host.docker.internal` proxy ports as the explicit network path instead of assuming container traffic is transparently tunneled.
- When Android tooling runs under standard proxy environment variables such as `HTTP_PROXY`, `HTTPS_PROXY`, or `ALL_PROXY`, the CLI must translate those values into `sdkmanager` native flags (`--proxy`, `--proxy_host`, `--proxy_port`) before launching Java, because `sdkmanager` cannot parse schemes like `socks5h://` from the raw environment.
- Android Gradle invocations must also translate proxy environment into Java/Gradle system properties. Passing only raw proxy env or only `GRADLE_OPTS` is not reliable enough for Gradle daemon dependency resolution.
- Native ARM Linux containers still consume Google's Linux `platform-tools` and NDK host binaries, which are `x86_64`. `water doctor --fix` must install the required `x86_64` userspace compatibility libraries on apt-based ARM Linux instead of telling the user to switch to an amd64 container.

## CRITICAL: Reactive-First Pattern

**WaterUI is a reactive framework. ALWAYS pass Bindings directly to APIs instead of using `.get()` or `watch`.**

Most WaterUI APIs accept `impl Signal` or `impl IntoSignalF32` - pass bindings directly for automatic reactivity:

```rust
// ✅ CORRECT - Pass binding directly, updates automatically
Photo::new(url).blur(blur_value.clone())       // blur updates as slider moves
view.visible(is_visible.clone())               // visibility reacts to state
view.opacity(opacity_value.clone())            // opacity animates reactively
view.disabled(is_loading.clone())              // disabled state follows loading
text!("Count: {count}")                        // text updates automatically

// ❌ WRONG - Static value, requires manual refresh
Photo::new(url).blur(blur_value.get())         // blur frozen at initial value
view.visible(is_visible.get())                 // visibility never changes
watch(count.clone(), |c| text(format!("{c}"))) // unnecessary indirection
```

**Rule: If an API accepts a value that might change, check if it accepts `impl Signal` and pass the binding.**

## Quick Start

```rust
use waterui::prelude::*;

fn main() -> impl View {
    let count = Binding::i32(0);

    vstack((
        text!("Count: {count}").headline(),
        button("+1")
            .with_state(&count)
            .action(|c| c.set(c.get() + 1)),
    ))
}
```

## Views

Functions and closures are views:
```rust
fn card(title: &str) -> impl View {
    vstack((text(title).title(), Divider))
}

// Use directly - no wrapper needed
vstack((card("Hello"), card("World")))
```

Conditional rendering:
```rust
// Show or hide (Option<impl View> is a View)
is_new.map(|b| b.then(|| badge("New")))

// Binary choice (if-else)
when(is_logged_in, || dashboard()).otherwise(|| login_form())

// Multi-branch (if-elif-else)
when(state.equal_to(0), || "Loading")
    .or(state.equal_to(1), || "Ready")
    .otherwise(|| "Error")
```

## State

```rust
// Use type-specific constructors (Binding::new does NOT exist)
let toggle = Binding::bool(false);
let count = Binding::i32(0);
let value = Binding::f64(1.5);
let name = Binding::container(String::new());  // heap types (String, Vec, etc.)
let text = Binding::container(Str::from("hello")); // Str type

// Pass by reference to child views
fn section(count: &Binding<i32>) -> impl View { ... }
```

## Reactive Transforms

Methods on signals (no `.clone()` needed for transforms):
```rust
count.not()                    // bool negation
count.select(a, b)             // if-else
count.equal_to(5)              // equality check
count.gt(0)                    // comparisons: lt, le, ge
count.is_empty()               // for strings/collections
count.map(|v| v * 2)           // custom transform
count.zip(&other).map(|(a,b)| a + b)  // combine signals
```

Convert to Computed: `signal.computed()`

## Reactive Modifiers

**Pass bindings directly to modifiers for real-time updates:**

```rust
let opacity = Binding::f64(1.0);
let blur = Binding::f64(0.0);
let is_visible = Binding::bool(true);
let is_disabled = Binding::bool(false);
let scale_factor = Binding::f64(1.0);

view
    .opacity(opacity.clone())           // reactive opacity
    .visible(is_visible.clone())        // reactive visibility
    .disabled(is_disabled.clone())      // reactive disabled state
    .scale(scale_factor.clone(), scale_factor.clone())  // reactive scale

// Filters also accept reactive values
Photo::new(url)
    .blur(blur.clone())                 // blur updates in real-time
    .saturation(saturation.clone())     // saturation updates in real-time
    .brightness(brightness.clone())     // brightness updates in real-time
```

## Event Handlers

**IMPORTANT: Always use `.with_state()` - never clone bindings manually!**

```rust
// Single state - receives Binding directly
button("Click")
    .with_state(&count)
    .action(|c| c.set(c.get() + 1))

// Multiple states → nested tuple (((a, b), c), d)
button("Reset")
    .with_state(&x)
    .with_state(&y)
    .action(|(x, y)| { x.set(0); y.set(0); })

// Four states example
button("Submit")
    .with_state(&url)
    .with_state(&blur)
    .with_state(&status)
    .with_state(&handler)
    .action(|(((url, blur), status), handler)| {
        // Use all four bindings
    })

// Async
button("Load").action_async(|_| async { fetch().await })

// Lifecycle
view.on_appear(|| setup())
view.on_change(&signal, |new_val| handle(new_val))
```

## Text

**IMPORTANT: Use `text()` for static text and `text!` for reactive text. Never use `watch()` just to build text. Also never write `waterui::text!`; import the macro and use `text!` directly.**

```rust
// Static text - use text() function
text("Hello").title()       // semantic sizes: title, headline, body, caption, footnote, sub_headline

// Reactive text - use text! macro (auto-updates when bindings change)
text!("Count: {count}")              // single binding
text!("{a} + {b} = {sum}")           // multiple bindings
text!("Value: {value:.2}")           // with formatting
text!("{FOCUSED_READOUT}")           // const &str capture is fine if text! behavior is desired

// text! returns LocalizedText with font methods
text!("Status: {status}").sub_headline()
text!("Small: {value}").caption()
```

## Layout

```rust
hstack((a, b, c)).spacing(8.0)
vstack((a, b)).padding()
zstack((background, content))
scroll(content)
spacer()                    // flexible space
spacer().height(16.0)       // fixed space

// From iterator - use .collect() for dynamic layouts
let buttons: HStack<_> = items.iter().map(|i| button(i.label)).collect();
```

## Colors

```rust
// Built-in (zero-sized, efficient)
Blue, Green, Red, Orange, Purple, Cyan, Yellow, Pink, Grey

// Custom
const BRAND: Srgb = Srgb::from_hex("#3B82F6");

// Usage - colors are Views
view.background(Blue)
view.foreground(BRAND)
Blue.size(80.0, 80.0)       // colored rectangle
BRAND.with_opacity(0.5)
```

Theme colors: `Foreground`, `MutedForeground`, `Accent`, `Background`, `Surface`, `Border`

## Modifiers

```rust
.padding() / .padding_with(EdgeInsets::all(16.0))
.background(color) / .foreground(color)
.size(w, h) / .width(w) / .height(h)
.scale(x, y) / .rotation(degrees) / .offset(x, y)
.border(color, width) / .shadow() / .clip(shape)
.disabled(bool_signal) / .visible(bool_signal)  // accept signals!
.opacity(f64_signal)                             // accepts signal!
```

## Components

| Category | Components |
|----------|------------|
| Layout | `hstack`, `vstack`, `zstack`, `scroll`, `spacer`, `grid` |
| Controls | `button`, `toggle`, `Slider`, `Stepper`, `TextField`, `Menu` |
| Navigation | `NavigationStack`, `NavigationLink`, `TabView` |
| Media | `Photo`, `VideoPlayer`, `MediaPicker` |
| Graphics | `Canvas`, `Chart`, `Map`, `Barcode::qr()` |

## CLI Commands

```bash
water create my-app              # new project
water run --platform ios         # run on simulator
water run --platform android
water run --platform macos
water preview my_view            # preview #[preview] function
water run --logs debug           # with debug output
```

## Preview System

Use the `#[preview]` macro to enable instant view previews:

- For playground projects, preview dylibs are built from the managed `ffi` wrapper crate, not from the user example crate directly. Keep playground example crates as plain Rust `lib` crates; do not add `crate-type` just for preview or native packaging.

```rust
#[preview]
fn my_card() -> impl View {
    text!("Hello Preview!")
}
```

**For visual verification, use the `waterui-preview` subagent** via the Task tool:

```
Task(subagent_type="waterui-preview", prompt="<function_name> --platform macos --path <crate_path>\nExpect: <visual description>")
```

## Common Patterns

```rust
// Reactive blur with slider (real-time updates)
let blur = Binding::f64(0.0);
vstack((
    Photo::new(url).blur(blur.clone()),  // blur reacts to slider
    Slider::new(0.0..=10.0, &blur),
    text!("Blur: {blur:.1}"),
))

// Animated toggle
let scale = active.select(1.2 as f32, 1.0).with(Animation::spring(300.0, 15.0));

// Conditional visibility (reactive)
.visible(items.map(|i| !i.is_empty()).computed())

// List rendering
List::for_each(&items, |item| item_view(item))

// Static layout from slice/array via FromIterator
fn tab_buttons(tabs: &[Tab], selected: &Binding<Tab>) -> HStack<(Vec<AnyView>,)> {
    tabs.iter()
        .map(|&tab| button(tab.label()).with_state(selected).action(move |s| s.set(tab)))
        .collect()
}

// Conditional views - prefer when().otherwise() over match
when(is_dark, || dark_theme()).otherwise(|| light_theme())
when(!is_loading, || content()).otherwise(|| spinner())

// Multi-branch conditionals
when(state.equal_to(0), || loading_view())
    .or(state.equal_to(1), || ready_view())
    .or(state.equal_to(2), || error_view())
    .otherwise(|| unknown_view())

// For many branches or complex matching, use match + .anyview()
fn render(mode: Mode) -> AnyView {
    match mode {
        Mode::A => view_a().anyview(),
        Mode::B => view_b().anyview(),
        Mode::C => view_c().anyview(),
    }
}

// Form from struct
#[derive(FormBuilder)]
struct Settings { name: String, volume: f64 }
form(&settings_binding)

// Dynamic view for URL changes (Photo with reactive blur)
let url_input = Binding::container(Str::from("https://example.com/image.jpg"));
let blur = Binding::f64(0.0);
let status = Binding::container(String::from("Loading..."));
let (handler, photo_view) = Dynamic::new();

// Load button - only Dynamic for URL change, blur is reactive
button("Load")
    .with_state(&url_input)
    .with_state(&blur)
    .with_state(&status)
    .with_state(&handler)
    .action(|(((url, blur), status), handler)| {
        let photo = Photo::new(url.get())
            .on_event({
                let status = status.clone();
                move |event| match event {
                    PhotoEvent::Loaded => status.set(String::from("Loaded")),
                    PhotoEvent::Error(msg) => status.set(format!("Error: {msg}")),
                }
            })
            .blur(blur.clone());  // Pass binding for reactive blur!
        handler.set(photo);
    });

vstack((
    text!("{status}"),
    photo_view,
    Slider::new(0.0..=10.0, &blur),  // Slider controls blur in real-time
))
```

## Extension Traits

WaterUI uses `*Ext` traits. When unsure, search `trait.*Ext` in codebase.

**SignalExt** (from nami, works on `Binding`/`Computed`):
```rust
// Core
.map(|v| ...), .zip(&other), .computed(), .cached(), .distinct(), .with(metadata)

// Bool → Signal<bool>
.not(), .select(if_true, if_false), .then_some(value)

// Comparison → Signal<bool>
.equal_to(v), .gt(v), .lt(v), .ge(v), .le(v), .condition(|v| ...)

// Option<T>
.is_some(), .is_none(), .unwrap_or(default), .map_some(|v| ...)

// String
.is_empty(), .contains("pattern")
```

**ViewExt**: `.anyview()`, `.visible()`, `.padding()`, `.background()`, etc.

**AnimationExt**: `.animated()`, `.with(Animation::spring(...))`

## Gotchas

**No `Binding::new()`** - use type-specific constructors:
```rust
// WRONG
let count = Binding::new(0);

// CORRECT
let count = Binding::i32(0);
let value = Binding::f64(1.5);
let flag = Binding::bool(false);
let name = Binding::container(String::new());
```

**No `_f32` suffix** - use `as f32` cast:
```rust
// WRONG
.select(1.0_f32, 0.3)

// CORRECT
.select(1.0 as f32, 0.3)
```

**No `.get()` for reactive values** - pass binding directly:
```rust
// WRONG - static, won't update
Photo::new(url).blur(blur.get())
view.opacity(opacity.get())

// CORRECT - reactive, updates automatically
Photo::new(url).blur(blur.clone())
view.opacity(opacity.clone())
```

**No `watch()` for text**:
```rust
// WRONG
watch(status.clone(), |msg| text(msg))

// CORRECT: reactive
text!("{status}")

// CORRECT: static
text("Ready")
```

**No `watch()` when reactive API exists** - pass binding directly:
IMPORTANT: `watch` would rebuild the entire subtree on every change and lost internal state, only a few scenarios require `watch`, always check if the API accepts `impl Signal` first
```rust
// WRONG - unnecessary watch
watch(blur.clone(), |b| Photo::new(url).blur(b))

// CORRECT - pass binding directly
Photo::new(url).blur(blur.clone())
```

**No manual `.clone()` for button states** - use `.with_state()`:
```rust
// WRONG
let count_clone = count.clone();
button("Click").action(move || count_clone.set(...))

// CORRECT
button("Click").with_state(&count).action(|c| c.set(...))
```

**Two-param transforms:**
```rust
.scale(x, y)    // not .scale(uniform)
.offset(x, y)
.size(w, h)
```

**`text!` returns `LocalizedText`** - supports all font methods:
```rust
// LocalizedText has: .title(), .headline(), .sub_headline(), .body(), .caption(), .footnote()
// Plus: .size(), .bold(), .italic(), .font()
text!("{status}").sub_headline()
text!("{value}").caption()
text!("{note}").footnote()
```

**Linux all-features CI native dependencies** - backend-enabled jobs need the native link headers for every compiled backend:
```text
libasound2-dev libva-dev libfontconfig1-dev libgbm-dev libxcb1-dev libglib2.0-dev libpango1.0-dev libgdk-pixbuf-2.0-dev libgtk-4-dev
```
Keep these packages installed in Linux lint, coverage, feature-check, and test jobs instead of disabling backend features to make CI pass.

**Windows all-features dylib linking** - `waterui/dynamic_linking` is a real preview path and must remain available for Apple and Android preview builds. Do not make `waterui-dylib` Apple-only or remove the dylib crate from supported preview platforms. Windows is not a WaterUI preview target today, and PE/COFF cannot represent the full Rust `dylib` export set for the all-features anchor crate; keep the optional `waterui-dylib` dependency target-gated away from Windows while preserving `dynamic_linking` for macOS, iOS, and Android. Windows debug all-features builds should still use the Rust toolchain's LLD PE/COFF linker (`rust-lld` with `lld-link` flavor) so other large debug links avoid MSVC `link.exe` object-count limits.

**Release scaffold backend refs** - when a backend submodule pointer changes, update the matching `cli/Cargo.toml` `package.metadata.waterui-scaffold.*-backend-commit` entry in the same superproject commit. The CLI build script derives backend refs from live submodules for dev builds, but release/package fallback metadata must stay pinned to the exact submodule commits so clean package verification and coverage agree with the source tree.

**Release package dependency graph** - every normal dependency of a release-plz-managed package must either already exist on crates.io or be listed as its own release package before dependents that need it. Local-only test harness crates such as `waterui-testing` must stay path-only when used as dev-dependencies so Cargo strips them from packaged manifests instead of treating them as registry dependencies.
