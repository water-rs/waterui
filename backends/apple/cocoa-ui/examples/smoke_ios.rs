//! Smoke check: answer the connected window scene with a window whose
//! [`ViewController`] hosts a [`HostView`], run one layout pass, print what
//! the pass reported, and exit.

#[cfg(target_os = "ios")]
use cocoa_ui::MainThreadMarker;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{ApplicationHandlers, ViewController, Window, run};

#[cfg(target_os = "ios")]
fn main() {
    let mtm = MainThreadMarker::new().expect("main runs on the main thread");
    run(
        mtm,
        ApplicationHandlers::new(|scene| {
            println!("smoke-ios: scene connected");
            let window = Window::new(scene);
            let controller = ViewController::new(scene.main_thread());
            controller.host_view().set_layout_handler(|view| {
                println!(
                    "smoke-ios: layout pass (safe_area_insets={:?}, display_scale={:?})",
                    view.safe_area_insets(),
                    view.display_scale()
                );
                println!("smoke-ios: PASS");
                std::process::exit(0);
            });
            window.set_root_view_controller(&controller);
            window.make_key_and_visible();
            println!("smoke-ios: window visible");
            window
        }),
    );
}

#[cfg(not(target_os = "ios"))]
fn main() {}
