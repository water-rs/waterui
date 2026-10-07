//! Mounting the application's own `Window` — the mount the window runner
//! performs — means the runtime's window is the app's: `Window::frame` is
//! written from the viewport at mount and on every `Moved`/`Resized` event,
//! and `Window::state` changes land on the app's bindings — #128. A synthetic
//! default window would orphan all of it.

use super::{MinimalTestTheme, test_environment};
use nami::Signal as _;
use waterui::prelude::*;
use waterui::window::{Window, WindowState};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::layout::{Point, Rect, Size};
use waterui_core::{AnyView, Binding, binding};

use crate::InputEvent;

#[test]
fn the_mounted_window_is_the_apps_window() {
    let env = test_environment();
    let frame = binding(Rect::new(Point::new(4.0, 6.0), Size::new(800.0, 600.0)));
    let state = binding(WindowState::Normal);
    let content = AnyViewBuilder::<AnyView>::new(move || AnyView::new(vstack(((),))));
    let mut window = Window::new("app window", state.clone(), move || content.build());
    window.frame = frame.clone();
    let mut rt = crate::HeadlessRuntime::new_for_tests_with_window(
        env,
        window,
        320,
        240,
        MinimalTestTheme::default(),
    );

    rt.pump_offscreen();
    assert_eq!(
        frame.snapshot(),
        Rect::new(Point::zero(), Size::new(320.0, 240.0)),
        "the viewport must write the app's Window::frame at mount"
    );

    rt.push_input_event(InputEvent::Resize {
        width: 640,
        height: 480,
    });
    rt.pump_offscreen();
    assert_eq!(
        *frame.snapshot().size(),
        Size::new(640.0, 480.0),
        "a viewport resize must rewrite the app's Window::frame"
    );

    rt.push_input_event(InputEvent::Moved { x: 12.0, y: 8.0 });
    rt.pump_offscreen();
    assert_eq!(
        frame.snapshot().origin(),
        Point::new(12.0, 8.0),
        "a window move must rewrite the app's Window::frame"
    );

    rt.push_input_event(InputEvent::CloseRequested);
    rt.pump_offscreen();
    assert_eq!(
        state.snapshot(),
        WindowState::Closed,
        "a close request must land on the app's Window::state"
    );
}

/// `Window::closable` gates the one close path: a close request leaves a
/// non-closable window open — its state untouched and its frame still
/// tracking the viewport — while the same request closes a closable one.
/// X11 and Wayland keep the title-bar button enabled whatever `closable`
/// says, so this gate is all that stands between the button and the close.
#[test]
fn a_close_request_closes_only_a_closable_window() {
    fn runtime_for(
        closable: bool,
    ) -> (crate::HeadlessRuntime, Binding<WindowState>, Binding<Rect>) {
        let state = binding(WindowState::Normal);
        let content = AnyViewBuilder::<AnyView>::new(move || AnyView::new(vstack(((),))));
        let mut window = Window::new("app window", state.clone(), move || content.build());
        window.closable = closable;
        let frame = window.frame.clone();
        let rt = crate::HeadlessRuntime::new_for_tests_with_window(
            test_environment(),
            window,
            320,
            240,
            MinimalTestTheme::default(),
        );
        (rt, state, frame)
    }

    let (mut rt, state, frame) = runtime_for(false);
    rt.pump_offscreen();
    rt.push_input_event(InputEvent::CloseRequested);
    rt.pump_offscreen();
    assert_eq!(
        state.snapshot(),
        WindowState::Normal,
        "a non-closable window must keep its state through a close request"
    );
    rt.push_input_event(InputEvent::Resize {
        width: 640,
        height: 480,
    });
    rt.pump_offscreen();
    assert_eq!(
        *frame.snapshot().size(),
        Size::new(640.0, 480.0),
        "a non-closable window must stay open and mounted after a close request"
    );

    let (mut rt, state, _frame) = runtime_for(true);
    rt.pump_offscreen();
    rt.push_input_event(InputEvent::CloseRequested);
    rt.pump_offscreen();
    assert_eq!(
        state.snapshot(),
        WindowState::Closed,
        "a closable window must close on a close request"
    );
}
