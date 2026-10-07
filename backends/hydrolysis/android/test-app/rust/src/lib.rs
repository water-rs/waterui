//! The debug app the Android landing tests run: a scrollable column of
//! text, a counter driven by a real tap, a toggle, a slider, a text field
//! and a secure field for IME/accessibility/autofill coverage, plus one
//! `Native<PlatformView>` leaf that mounts the registry's `WebView` between
//! GPU-drawn controls for the z-order and event-ownership checks.
//!
//! A workspace member, so host cargo sweeps reach it: its only artifact is
//! the cdylib the Kotlin host loads, and every symbol it names (jni,
//! `hydrolysis::android`) exists on Android alone — off the target the crate
//! is empty by contract.
#![cfg(target_os = "android")]

use std::rc::Rc;

use waterui::app::App;
use waterui::component::text_field::{ContentType, KeyboardType};
use waterui::form::secure::Secure;
use waterui::graphics::color::Srgb;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::window::{Window, WindowState};
use waterui_core::Native;

/// The bindings arrive owned so the returned view holds no borrow of the
/// window closure's locals; each is only borrowed in the body, which the
/// pedantic lint reads as `needless_pass_by_value`.
#[allow(clippy::needless_pass_by_value)]
fn app_view(
    count: Binding<i32>,
    enabled: Binding<bool>,
    name: Binding<Str>,
    code: Binding<Str>,
    password: Binding<Secure>,
    volume: Binding<f64>,
) -> impl View {
    scroll(
        vstack((
            text("Hydrolysis on Android").title(),
            "Cherenkov paints this UI through the Kotlin host's SurfaceView band.",
            // Keeps the pump visibly pumping: the Material indeterminate
            // indicator animates on the shared animation clock, so logcat
            // shows `frame presented` lines while the window is visible and
            // none while it is hidden.
            loading(),
            Divider,
            text!("Tap count: {count}", count = count),
            button("Increment")
                .action(move |State(count): State<Binding<i32>>| {
                    count.with_mut(|value| *value += 1);
                })
                .state(&count),
            Toggle::new("Enable notifications", &enabled),
            text!("Notifications: {enabled}", enabled = enabled),
            slider("Volume", &volume),
            text!("Volume: {volume}", volume = volume),
            // The autofill/IME cluster nests: the outer column would otherwise
            // exceed `TupleViews`' arity once every control the Android landing
            // tests drive sits in a single tuple.
            vstack((
                TextField::new("Email address", &name)
                    .prompt("Autofill: email")
                    .content_type(ContentType::EmailAddress),
                TextField::new("Verification code", &code)
                    .keyboard(KeyboardType::Number)
                    .content_type(ContentType::OneTimeCode),
                SecureField::new("Password", &password),
                text!("Hello, {name}!", name = name),
            )),
            Divider,
            layout::frame::Frame::new(Native::new(hydrolysis::PlatformView::new("webview")))
                .height(220.0),
            text("Resize + recreation keep this tree alive.").foreground(Srgb::from_hex("#6B6B70")),
        ))
        .padding(),
    )
}

fn build_app() -> App {
    App::new_with_windows(
        [Window::new(
            "Hydrolysis Test App",
            binding(WindowState::Normal),
            move || {
                let count = binding(0);
                let enabled = binding(false);
                let name = binding("");
                let code = binding("");
                let password = binding(waterui::form::secure::Secure::new(String::new()));
                let volume = binding(0.5);
                app_view(count, enabled, name, code, password, volume)
            },
        )],
        Environment::new(),
    )
}

/// Entry from the JVM: registers the app factory the Kotlin host mounts per
/// session create. Logging installs at `nativeInit`, fed the launch intent's
/// `waterui.log.level` extra.
#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(
    _vm: *mut std::ffi::c_void,
    _reserved: *mut std::ffi::c_void,
) -> i32 {
    hydrolysis::android::register_app(|| {
        (
            build_app(),
            Rc::new(hydrolysis_m3::Material3::defaults()) as Rc<dyn hydrolysis::Style>,
        )
    });
    jni::sys::JNI_VERSION_1_6
}