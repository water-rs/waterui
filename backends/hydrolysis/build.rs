//! Build-time cfg aliases for the backend's platform and runner gates.

use cfg_aliases::cfg_aliases;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // An Android build with neither painter feature links nothing that can
    // draw: warn rather than error so the bare `cargo check --no-default-features`
    // lint leg still exercises the crate. The generated launcher always
    // passes the resolved painter's feature, so reaching this warns a
    // hand-rolled consumer.
    if std::env::var_os("CARGO_CFG_TARGET_OS").as_deref() == Some(std::ffi::OsStr::new("android"))
        && std::env::var_os("CARGO_FEATURE_GPU").is_none()
        && std::env::var_os("CARGO_FEATURE_HWUI").is_none()
    {
        println!(
            "cargo:warning=hydrolysis: Android build has no painter — enable the `gpu` or `hwui` feature; this build ships no painter"
        );
    }

    cfg_aliases! {
        // Where hosted content exists: the targets on which `cherenkov-gpu`
        // implements `HostedLayers`, whose plane shows a platform object.
        hydrolysis_hosted: { any(target_vendor = "apple", target_os = "android", target_arch = "wasm32") },
        apple: { any(target_os = "ios", target_os = "macos") },
        android_platform: { target_os = "android" },
        free_unix: { all(unix, not(apple), not(android_platform), not(target_os = "emscripten")) },
        // The winit runner exists on every target except Android: there the
        // Activity belongs to the Kotlin host, and winit's `android-activity`
        // can only build once an application selects a backend — nothing in
        // this library may. Every winit-scoped item gates on this alias, so
        // `--features winit` compiles on Android and simply means nothing.
        hydrolysis_winit: { all(feature = "winit", not(android_platform)) },
        // Where `hydrolysis::run` exists: every target but Android, whose
        // entry point is the Kotlin host, and bare wasm32, which reaches no
        // windowing model until the browser runner is compiled in. The
        // examples that need the runner gate on it too, so they and the
        // export never disagree.
        hydrolysis_run: {
            any(
                all(not(target_arch = "wasm32"), not(android_platform)),
                all(target_arch = "wasm32", feature = "web")
            )
        },
        // Where the engine presents a window itself, through a
        // `cherenkov_gpu::WindowTarget` over a view the host hands it. Today
        // that is the macOS winit window; it is the one gate the engine
        // window path keys on, so a host that presents the same way joins by
        // widening this alias.
        hydrolysis_apple_engine_window: { all(hydrolysis_winit, target_os = "macos") },
        // The platform range-editing protocol (`runner::editing`): a host
        // whose input method edits the focused field through UTF-16 ranges.
        // Test builds compile it too, but a build script cannot see `test`,
        // so that arm sits at the gate, beside this alias.
        hydrolysis_range_editing: { android_platform },
        hydrolysis_wayland_platform: { all(feature = "winit", free_unix, not(target_os = "redox")) },
        // The winit desktops, whose windows the application closes itself
        // (Close Window, ⌘W/Ctrl+W). On iOS and the web the system or the
        // page owns the window, so the runner offers no close primitive.
        hydrolysis_closable_windows: {
            all(hydrolysis_winit, any(target_os = "macos", target_os = "windows", free_unix))
        },
        // The winit desktops whose host answers the runner's direct queries:
        // the live pointer position, a monitor's work area, and the
        // activation attributes winit leaves unset. iOS has none of them.
        hydrolysis_desktop_queries: {
            any(target_os = "macos", target_os = "windows", hydrolysis_wayland_platform)
        },
        // The macOS `WKWebView` bridge needs a real window: it is hosted on a
        // compositor plane under the winit window's AppKit view, so a headless
        // build (the renderer `waterui-testing` drives, a `web` build) has
        // nowhere to put it and compiles none of it. The feature alone is not
        // enough, which is why this alias exists and the module turns an
        // unsatisfiable request into a `compile_error!` that says what to enable.
        hydrolysis_macos_system_webview: {
            all(feature = "winit", target_os = "macos", feature = "webview-system")
        },
        // The Android `WebView` bridge needs no window of its own: the view is
        // a child of the Kotlin host's platform-view overlay, so the feature
        // alone is enough here — the winit gate that shapes the macOS alias
        // simply does not exist on Android.
        hydrolysis_android_system_webview: {
            all(target_os = "android", feature = "webview-system")
        },
        // On the web the page's own web view is an `<iframe>`, hosted on a DOM
        // plane under the page's root element.
        hydrolysis_web_system_webview: {
            all(target_arch = "wasm32", feature = "web", feature = "webview-system")
        },
        // Either platform bridge: where `WebView` reaches a web engine this
        // backend bridges.
        hydrolysis_system_webview: {
            any(hydrolysis_macos_system_webview, hydrolysis_web_system_webview)
        },
        // The persistent pipeline cache serialises through `std::fs` into a
        // platform cache directory; targets without that filesystem contract
        // (wasm, redox) compile the renderer without it.
        hydrolysis_pipeline_cache: { any(unix, windows) },
        // The Android HWUI render target: the retained layer tree recorded
        // into `RenderNode` display lists, on Android only.
        hydrolysis_hwui: { all(feature = "hwui", android_platform) },
        // Runtime implementation exists only when a painter is selected.
        hydrolysis_runtime: { any(hydrolysis_gpu_painter, hydrolysis_hwui) },
        // The Cherenkov GPU engine, its wgpu device stack and the parley
        // text engine are linkable: always off-Android (one painter, the
        // deps unconditional), on Android only under the `gpu` feature so a
        // pure-HWUI build links none of it. The GPU *painter* machinery is
        // the narrower `hydrolysis_gpu_painter` gate this alias is not: under
        // `hwui` + `gpu` the deps and the GPU-content surface exist while
        // the painter stays HWUI.
        hydrolysis_gpu: { any(not(android_platform), feature = "gpu") },
        // The GPU painter machinery — wgpu surface negotiation, the
        // compositor, the Parley engine, the GPU window traits: present
        // where the Cherenkov GPU attachment is the painter, i.e. every
        // non-Android target and Android's `gpu`-without-`hwui` shape. An
        // Android build with neither feature compiles no painter at all —
        // the bare `cargo check` shape CI lints — so this is a real gate,
        // not `not(hydrolysis_hwui)` restated.
        hydrolysis_gpu_painter: {
            any(not(android_platform), all(feature = "gpu", not(feature = "hwui")))
        },
    }
}
