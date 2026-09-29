//! The debug app the Android landing tests run: a scrollable column of
//! text, a counter driven by a real tap, a toggle and a text field — enough
//! to show a rendered UI, redraw on interaction, resize on recreation, and
//! idle with no pumping between frames.

use std::rc::Rc;

use waterui::app::App;
use waterui::graphics::color::Srgb;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::window::{Window, WindowState};

fn app_view(count: Binding<i32>, enabled: Binding<bool>, name: Binding<Str>) -> impl View {
    scroll(
        vstack((
            text("Hydrolysis on Android").title(),
            "Vello paints this UI through the Kotlin host's SurfaceView band.",
            Divider,
            text!("Tap count: {count}", count = count),
            button("Increment")
                .action(move |State(count): State<Binding<i32>>| {
                    count.with_mut(|value| *value += 1);
                })
                .state(&count),
            Toggle::new("Enable notifications", &enabled),
            text!("Notifications: {enabled}", enabled = enabled),
            TextField::new("Your name", &name).prompt("Type to drive the IME"),
            text!("Hello, {name}!", name = name),
            Divider,
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
                app_view(count, enabled, name)
            },
        )],
        Environment::new(),
    )
}

/// Entry from the JVM: installs logging, registers the app factory the
/// Kotlin host mounts per session create.
#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(
    _vm: *mut std::ffi::c_void,
    _reserved: *mut std::ffi::c_void,
) -> i32 {
    hydrolysis::android::init_logging();
    hydrolysis::android::register_app(|| {
        (
            build_app(),
            Rc::new(hydrolysis_m3::Material3::defaults()) as Rc<dyn hydrolysis::Style>,
        )
    });
    jni::sys::JNI_VERSION_1_6
}
