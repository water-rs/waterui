//! Probe for the runtime window API: asserts the window handle, scale factor,
//! and title plumbing through the platform runner.

use std::time::Duration;

use hydrolysis::run;
use waterui::Environment;
use waterui::app::App;
use waterui::layout::Size;
use waterui::prelude::*;
use waterui::task::sleep;
use waterui::window::{UserAttention, Window, WindowHandle, WindowLevel, WindowState};

/// Live probe for the window API (water-rs/waterui#1264, #1268): the window
/// opens always-on-top with 24x12 resize increments, maximizes itself after a
/// beat, then raises a critical attention request. On X11 `xprop` against the
/// window shows `_NET_WM_STATE_ABOVE`, `_NET_WM_STATE_MAXIMIZED_*`,
/// `_NET_WM_STATE_DEMANDS_ATTENTION`, and the increment in `WM_NORMAL_HINTS`.
fn main() {
    let mut window = Window::new(
        "window-api-probe",
        waterui::reactive::binding(WindowState::Normal),
        || (),
    )
    .level(WindowLevel::AlwaysOnTop)
    .resize_increments(Size::new(24.0, 12.0));

    let handle: WindowHandle = window.handle();
    window.content = waterui::handler::AnyViewBuilder::new(move || {
        AnyView::new(text("window API probe").padding().task({
            let handle = handle.clone();
            async move {
                sleep(Duration::from_secs(4)).await;
                handle.maximize();
                sleep(Duration::from_secs(1)).await;
                handle.request_attention(UserAttention::Critical);
                sleep(Duration::from_secs(20)).await;
                std::process::exit(0);
            }
        }))
    });

    run(
        App::new_with_windows([window], Environment::new()),
        hydrolysis_m3::Material3::defaults(),
    );
}
