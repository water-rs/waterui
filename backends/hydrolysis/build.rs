use cfg_aliases::cfg_aliases;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    cfg_aliases! {
        apple: { any(target_os = "ios", target_os = "macos") },
        android_platform: { target_os = "android" },
        free_unix: { all(unix, not(apple), not(android_platform), not(target_os = "emscripten")) },
        hydrolysis_wayland_platform: { all(feature = "winit", free_unix, not(target_os = "redox")) },
    }
}
