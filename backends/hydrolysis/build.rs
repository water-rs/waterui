//! Build-time cfg aliases for the backend's platform and runner gates.

use cfg_aliases::cfg_aliases;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    cfg_aliases! {
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
        hydrolysis_wayland_platform: { all(feature = "winit", free_unix, not(target_os = "redox")) },
        // The winit desktops, whose windows the application closes itself
        // (Close Window, ⌘W/Ctrl+W). On iOS and the web the system or the
        // page owns the window, so the runner offers no close primitive.
        hydrolysis_closable_windows: {
            all(hydrolysis_winit, any(target_os = "macos", target_os = "windows", free_unix))
        },
        // The macOS `WKWebView` bridge needs a real window: it is composed into
        // the winit window's AppKit view as a native subview, so a headless
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
        // The persistent pipeline cache serialises through `std::fs` into a
        // platform cache directory; targets without that filesystem contract
        // (wasm, espidf, redox) compile the renderer without it.
        hydrolysis_pipeline_cache: { any(unix, windows) },
    }
}
