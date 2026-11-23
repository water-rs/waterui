//! Build-time cfg aliases for the backend's platform and runner gates.

use cfg_aliases::cfg_aliases;

#[cfg(feature = "hwui")]
#[path = "build/hwui.rs"]
mod hwui;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    facade_features();
    #[cfg(feature = "hwui")]
    {
        for input in [
            "build/hwui.rs",
            "src/hwui/codes.rs",
            "src/hwui/protocol.rs",
            "src/hwui/text/constants.rs",
            "src/hwui/kotlin.rs",
            "templates/hwui/Protocol.kt",
        ] {
            println!("cargo:rerun-if-changed={input}");
        }
        let generated =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"))
                .join("hwui-kotlin");
        hwui::kotlin::generate(&generated);
        println!(
            "cargo:rustc-env=WATERUI_HWUI_KOTLIN_DIR={}",
            generated.display()
        );
    }
    // A build with no painter feature links nothing that can draw: warn
    // rather than error so the bare `cargo check --no-default-features` lint
    // leg still exercises the crate. The generated launcher always passes
    // the resolved painter features, so reaching this warns a hand-rolled
    // consumer. `hwui` paints only on Android.
    let android =
        std::env::var_os("CARGO_CFG_TARGET_OS").as_deref() == Some(std::ffi::OsStr::new("android"));
    if std::env::var_os("CARGO_FEATURE_GPU").is_none()
        && std::env::var_os("CARGO_FEATURE_CPU").is_none()
        && !(android && std::env::var_os("CARGO_FEATURE_HWUI").is_some())
    {
        println!(
            "cargo:warning=hydrolysis: this build has no painter — enable the `gpu` or `cpu` feature (or `hwui` on Android); this build ships no painter"
        );
    }

    // `trybuild` builds every crate of its scratch project with
    // `--cfg trybuild -A dead_code`, so a `dead_code` expectation cannot be
    // fulfilled there.
    println!("cargo:rustc-check-cfg=cfg(trybuild)");
    configure_cfg_aliases();
    configure_standalone_aliases();
}

fn configure_cfg_aliases() {
    cfg_aliases! {
        // Where hosted content exists: the targets on which `cherenkov-gpu`
        // implements `HostedLayers`, whose plane shows a platform object.
        hydrolysis_hosted: { any(target_vendor = "apple", target_os = "android", target_arch = "wasm32") },
        apple: { any(target_os = "ios", target_os = "macos") },
        android_platform: { target_os = "android" },
        // The UIKit host: iOS runs on `cocoa-ui`'s scene delegate and view,
        // never on winit, and presents through the GPU painter.
        hydrolysis_uikit: { all(target_os = "ios", feature = "gpu") },
        // The hosts that drive frames through the shared frame transaction
        // (`runner::window::run_frame_transaction`): the platform schedules
        // each frame itself — Choreographer, `CADisplayLink` — instead of
        // winit's event loop.
        hydrolysis_frame_transaction: { any(android_platform, hydrolysis_uikit) },
        free_unix: { all(unix, not(apple), not(android_platform), not(target_os = "emscripten")) },
        // The winit runner exists on every target except Android and iOS:
        // there the Activity belongs to the Kotlin host, and winit's
        // `android-activity` can only build once an application selects a
        // backend — nothing in this library may — and on iOS the UIKit host
        // owns the application. Every winit-scoped item gates on this alias,
        // so `--features winit` compiles on both and simply means nothing.
        // It presents through the GPU painter or, via `softbuffer`, the CPU
        // painter; `lib.rs` names the missing painter feature when `winit` is
        // asked for with neither.
        hydrolysis_winit: {
            all(feature = "winit", not(android_platform), not(target_os = "ios"), hydrolysis_cherenkov)
        },
        // Where `hydrolysis::run` exists: every target but Android, whose
        // entry point is the Kotlin host, and bare wasm32, which reaches no
        // windowing model until the browser runner is compiled in. The
        // examples that need the runner gate on it too, so they and the
        // export never disagree. A Cherenkov painter draws every runner's
        // frames.
        hydrolysis_run: {
            all(
                hydrolysis_cherenkov,
                any(
                    all(not(target_arch = "wasm32"), not(android_platform)),
                    all(target_arch = "wasm32", feature = "web")
                )
            )
        },
        // Where the engine presents a window itself, through a
        // `cherenkov_gpu::WindowTarget` over a view the host hands it. Today
        // that is the macOS winit window and every UIKit scene; it is the one
        // gate the engine window path keys on, so a host that presents the
        // same way joins by widening this alias.
        hydrolysis_apple_engine_window: {
            any(all(hydrolysis_winit, hydrolysis_gpu_painter, target_os = "macos"), hydrolysis_uikit)
        },
        // The platform range-editing protocol (`runner::editing`): a host
        // whose input method edits the focused field through UTF-16 ranges,
        // in a build that has a text field to edit. Test builds compile it
        // too, but a build script cannot see `test`, so that arm sits at the
        // gate, beside this alias.
        hydrolysis_range_editing: {
            all(
                any(android_platform, hydrolysis_uikit),
                any(feature = "text-field", feature = "secure-field")
            )
        },
        hydrolysis_wayland_platform: { all(hydrolysis_winit, free_unix, not(target_os = "redox")) },
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
            all(hydrolysis_winit, target_os = "macos", feature = "webview-system")
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
        // Bundled CEF engine realizing `WebView` on desktop targets.
        hydrolysis_cef_webview: {
            all(feature = "webview-cef", any(target_os = "macos", target_os = "linux", target_os = "windows"))
        },
        // Bundled WPE WebKit engine realizing `WebView` on Linux.
        hydrolysis_wpe_webview: {
            all(feature = "webview-wpe", target_os = "linux")
        },
        // Any non-Android web engine wired into `install_platform_services` and
        // `NativeBackend::web_view`.
        hydrolysis_webview_engine: {
            any(hydrolysis_system_webview, hydrolysis_cef_webview, hydrolysis_wpe_webview)
        },
        // The built-in localized strings: the controls that draw them, and the
        // list and navigation chrome that name accessibility nodes with them.
        hydrolysis_localization: {
            any(
                feature = "text-field",
                feature = "secure-field",
                feature = "color-picker",
                feature = "date-picker",
                feature = "navigation",
                all(feature = "list", feature = "accessibility")
            )
        },
        hydrolysis_platform_services: {
            any(hydrolysis_localization, hydrolysis_webview_engine)
        },
        // The Android HWUI render target: the retained layer tree recorded
        // into `RenderNode` display lists, on Android only.
        hydrolysis_hwui: { all(feature = "hwui", android_platform) },
        // The HWUI painter with GPU content: each GPU content view is hosted
        // in its own platform surface the painter composites in session
        // order (water-rs/waterui#1812).
        hydrolysis_hwui_gpu: { all(hydrolysis_hwui, feature = "gpu") },
        // The GPU video player's Android decoder plane: the backend realizes
        // `waterui-video-gpu`'s `AndroidVideoSurfaceHost` only when the
        // application links the player (the `video-gpu` feature).
        hydrolysis_video_plane: { all(android_platform, feature = "video-gpu") },
        // Runtime implementation exists only when a painter is selected.
        hydrolysis_runtime: { any(hydrolysis_cherenkov, hydrolysis_hwui) },
        // The Cherenkov GPU engine and its wgpu device stack are linkable:
        // the `gpu` feature on every target. The GPU *painter* machinery is
        // the narrower `hydrolysis_gpu_painter` gate this alias is not: under
        // `hwui` + `gpu` the deps and the GPU-content surface exist while
        // the painter stays HWUI.
        hydrolysis_gpu: { feature = "gpu" },
        // The GPU painter machinery — wgpu surface negotiation, the
        // compositor, the GPU window traits: present where the Cherenkov GPU
        // attachment is a painter, i.e. `gpu` everywhere but Android's
        // `hwui` shape. A build with no painter feature compiles no painter
        // at all — the bare `cargo check` shape CI lints — so this is a real
        // gate, not `not(hydrolysis_hwui)` restated.
        hydrolysis_gpu_painter: { all(feature = "gpu", not(hydrolysis_hwui)) },
        // The Cherenkov CPU painter: `cherenkov-cpu`'s rasterizer under the
        // offscreen runtime. Native targets only — the browser renders
        // through WebGPU — and never beside HWUI.
        hydrolysis_cpu_painter: {
            all(feature = "cpu", not(hydrolysis_hwui), not(target_arch = "wasm32"))
        },
        // Desktop software window presentation (`softbuffer`) for the CPU
        // painter: matches the exact target set declaring `softbuffer` in
        // `Cargo.toml` so `cargo::unused_dependencies` holds with or without
        // `winit`.
        hydrolysis_software_surface: {
            all(
                hydrolysis_cpu_painter,
                any(target_os = "macos", target_os = "linux", target_os = "windows", free_unix)
            )
        },
        // Either Cherenkov painter: the engine-generic mount targets, the
        // window frame commit and the parley text engine.
        hydrolysis_cherenkov: { any(hydrolysis_gpu_painter, hydrolysis_cpu_painter) },
        // Both Cherenkov painters: the offscreen runtime selects one at
        // construction from the host's adapters.
        hydrolysis_both_painters: { all(hydrolysis_gpu_painter, hydrolysis_cpu_painter) },
    }
}

fn configure_standalone_aliases() {
    cfg_aliases! {
        // The page's `<video>` element realizes `Video` on the web.
        hydrolysis_web_video: {
            all(target_arch = "wasm32", feature = "web", feature = "video")
        },
        // The GPU player realizes `Video` everywhere but the web: this
        // backend bridges no platform player.
        hydrolysis_gpu_video: {
            all(feature = "video-gpu", not(target_arch = "wasm32"))
        },
        // The persistent pipeline cache serialises through `std::fs` into a
        // platform cache directory; targets without that filesystem contract
        // (wasm, redox) compile the renderer without it.
        hydrolysis_pipeline_cache: { any(unix, windows) },
        // Development inspector endpoint: opt-in and unavailable on wasm32.
        hydrolysis_inspector: { all(feature = "inspector", not(target_arch = "wasm32")) },
    }
}

/// Turns the facade's enabled features into `waterui_feature` cfgs, and
/// fails the build when a facade component this crate realizes itself is on
/// without the feature that realizes it: `NativeBackend` then requires a
/// method this crate would not compile. Components drawn by a realization
/// the application links (`map`, `video`, a browser engine's `webview`)
/// are not listed: their methods report a missing realization at run time.
fn facade_features() {
    // Each facade feature, and the feature of this crate that realizes it.
    const REALIZED_BY: &[(&str, &str)] = &[
        ("calendar", "calendar"),
        ("color-picker", "color-picker"),
        ("date-picker", "date-picker"),
        ("multi-date-picker", "multi-date-picker"),
        ("picker", "picker"),
        ("secure-field", "secure-field"),
        ("slider", "slider"),
        ("stepper", "stepper"),
        ("text-field", "text-field"),
        ("toggle", "toggle"),
        ("menu", "menu"),
        ("list", "list"),
        ("table", "table"),
        ("navigation", "navigation"),
    ];

    println!("cargo::rustc-check-cfg=cfg(waterui_feature, values(any()))");
    println!("cargo::rustc-check-cfg=cfg(hydrolysis_scene_view)");
    let features = std::env::var("DEP_WATERUI_FEATURES")
        .expect("the `waterui` facade's build script exports DEP_WATERUI_FEATURES");
    let mut has_scene_view = false;
    for feature in features.split(',').filter(|feature| !feature.is_empty()) {
        println!("cargo::rustc-cfg=waterui_feature=\"{feature}\"");
        if matches!(feature, "canvas" | "gpu" | "cpu") {
            has_scene_view = true;
        }
        if let Some((_, own)) = REALIZED_BY.iter().find(|(name, _)| *name == feature) {
            let enabled = std::env::var_os(format!(
                "CARGO_FEATURE_{}",
                own.to_ascii_uppercase().replace('-', "_")
            ))
            .is_some();
            assert!(
                enabled,
                "the application enables the `waterui` feature `{feature}`; enable hydrolysis's `{own}` to realize it"
            );
        }
    }
    if has_scene_view {
        println!("cargo::rustc-cfg=hydrolysis_scene_view");
    }
}
