//! Smoke check: open one window hosting a [`HostView`], run one layout
//! pass, print what the pass reported, and exit.

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{
    ActivationPolicy, Application, ApplicationHandlers, HostView, Window, WindowStyle,
};
#[cfg(target_os = "macos")]
use cocoa_ui::{MainThreadMarker, Rect};

#[cfg(target_os = "macos")]
fn main() {
    let mtm = MainThreadMarker::new().expect("main runs on the main thread");
    let app = Application::shared(mtm);
    assert!(
        app.set_activation_policy(ActivationPolicy::Accessory),
        "activation policy switch was refused"
    );
    app.run(ApplicationHandlers::new().did_finish_launching(|mtm| {
        println!("smoke-macos: launched");
        let window = Window::new(
            mtm,
            Rect::new(100.0, 100.0, 640.0, 480.0),
            WindowStyle::all(),
        );
        window.set_title("cocoa-ui smoke");
        let host = HostView::new(mtm, window.content_rect());
        host.set_layout_handler(|view| {
            println!(
                "smoke-macos: layout pass (safe_area_insets={:?}, display_scale={:?})",
                view.safe_area_insets(),
                view.display_scale()
            );
            println!("smoke-macos: PASS");
            std::process::exit(0);
        });
        window.set_content_view(&host);
        window.make_key_and_order_front();
        host.set_needs_layout();
        host.layout_if_needed();
        std::mem::forget(window);
        println!("smoke-macos: layout handler never ran");
        std::process::exit(1);
    }));
}

#[cfg(not(target_os = "macos"))]
fn main() {}
