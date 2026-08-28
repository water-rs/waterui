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
        // The macOS `WKWebView` bridge needs a real window: it is composed into
        // the winit window's AppKit view as a native subview, so a headless
        // build (the renderer `waterui-testing` drives, a `web` build) has
        // nowhere to put it and compiles none of it. The feature alone is not
        // enough, which is why this alias exists and the module turns an
        // unsatisfiable request into a `compile_error!` that says what to enable.
        hydrolysis_macos_system_webview: {
            all(feature = "winit", target_os = "macos", feature = "webview-system")
        },
    }
}
