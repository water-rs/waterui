use cfg_aliases::cfg_aliases;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    waterui_build_support::shader::compile_wgsl_shader(
        manifest_dir.join("src/shaders/gpu_surface_compositor.wgsl"),
        "gpu_surface_compositor",
    );

    cfg_aliases! {
        apple: { any(target_os = "ios", target_os = "macos") },
        android_platform: { target_os = "android" },
        free_unix: { all(unix, not(apple), not(android_platform), not(target_os = "emscripten")) },
        hydrolysis_wayland_platform: { all(feature = "winit", free_unix, not(target_os = "redox")) },
        hydrolysis_macos_system_webview: {
            all(
                feature = "winit",
                target_os = "macos",
                any(feature = "webview-default", feature = "webview-system")
            )
        },
        hydrolysis_linux_wpe_webview: {
            all(
                target_os = "linux",
                any(feature = "webview-default", feature = "webview-wpe")
            )
        },
        hydrolysis_cef_webview: {
            all(
                feature = "webview-cef",
                any(target_os = "macos", target_os = "linux", target_os = "windows")
            )
        },
        hydrolysis_webview: {
            any(
                feature = "webview-default",
                feature = "webview-system",
                feature = "webview-wpe",
                feature = "webview-cef"
            )
        },
    }
}
