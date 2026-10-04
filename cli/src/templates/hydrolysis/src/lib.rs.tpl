//! Hydrolysis web entry point for {{ ctx.app_display_name }}.

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
use waterui::env::Environment;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() {
    let env = waterui::configure_environment!(Environment::new());
    let app = {{ ctx.crate_name_ident() }}::app(env);
    hydrolysis::run(app, hydrolysis_m3::Material3::defaults());
}

/// Android entry point: the Kotlin host's `NativeBridge.load` lands here and
/// registers the app factory each new session mounts. Logging installs at
/// `nativeInit` — the host hands it the launch intent's `waterui.log.level`
/// extra (the CLI's `--logs` level).
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(
    _vm: *mut std::ffi::c_void,
    _reserved: *mut std::ffi::c_void,
) -> i32 {
    use std::rc::Rc;

    hydrolysis::android::register_app(|| {
        let env = waterui::configure_environment!(waterui::env::Environment::new());
        (
            {{ ctx.crate_name_ident() }}::app(env),
            Rc::new(hydrolysis_m3::Material3::defaults()) as Rc<dyn hydrolysis::Style>,
        )
    });
    jni::sys::JNI_VERSION_1_6
}
