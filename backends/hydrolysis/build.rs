use cfg_aliases::cfg_aliases;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    shaderloom::build::compile_wgsl_shader(
        manifest_dir.join("src/shaders/gpu_surface_compositor.wgsl"),
        "gpu_surface_compositor",
    );

    cfg_aliases! {
        apple: { any(target_os = "ios", target_os = "macos") },
        android_platform: { target_os = "android" },
        free_unix: { all(unix, not(apple), not(android_platform), not(target_os = "emscripten")) },
        hydrolysis_wayland_platform: { all(feature = "winit", free_unix, not(target_os = "redox")) },
        // One rule for every engine: a windowed host, the target the engine runs
        // on, and a feature selecting it.
        //
        // The host requirement is `winit` for all three because every engine
        // needs somewhere real to put its pixels — the macOS engine embeds a
        // `WKWebView` as a native subview of the window, and the WPE and CEF
        // engines render into a `GpuSurface` registered on the window's node and
        // take pointer and keyboard input from it. A build with no window (the
        // headless renderer that `waterui-testing` drives, a `web` build) has
        // none of that, so an engine there would be code that cannot run.
        //
        // The rule used to differ per engine, and the difference was invisible
        // until it bit: the Linux WPE alias asked for no host at all, so a
        // headless build compiled the WPE engine, downcast whatever handle it
        // was given to `WpeWebViewHandle`, and panicked — on Linux only, while
        // macOS passed, purely because the macOS alias did require `winit`.
        // Keep these three in the same shape.
        hydrolysis_macos_system_webview: {
            all(
                feature = "winit",
                target_os = "macos",
                any(feature = "webview-default", feature = "webview-system")
            )
        },
        hydrolysis_linux_wpe_webview: {
            all(
                feature = "winit",
                target_os = "linux",
                any(feature = "webview-default", feature = "webview-wpe")
            )
        },
        hydrolysis_cef_webview: {
            all(
                feature = "winit",
                any(target_os = "macos", target_os = "linux", target_os = "windows"),
                feature = "webview-cef"
            )
        },
        // "A web engine resolved", not "a webview feature was asked for": the
        // features name bridges that only exist on some targets, so
        // `webview-system` off macOS or `webview-default` on Windows selects
        // nothing at all. Conflating the two gated the controller call on a
        // condition broader than the controller's own definition, and
        // `--features webview-system` failed to build with a missing
        // `install_controller`. `widgets/platform/webview.rs` turns each
        // unsatisfiable request into a `compile_error!` that says what to enable.
        hydrolysis_webview: {
            any(
                hydrolysis_macos_system_webview,
                hydrolysis_linux_wpe_webview,
                hydrolysis_cef_webview
            )
        },
        // The engines that take their input through the renderer's embedded
        // input targets, i.e. everything except the macOS system webview,
        // which is a native subview and receives input from AppKit itself.
        hydrolysis_browser_input: {
            any(
                hydrolysis_linux_wpe_webview,
                hydrolysis_cef_webview,
                feature = "chromium"
            )
        },
    }
}
