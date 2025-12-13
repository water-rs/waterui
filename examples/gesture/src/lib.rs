//! Gesture Example - Demonstrates WaterUI's gesture recognition capabilities
//!
//! This example showcases:
//! - Tap gestures (single and multi-tap)
//! - Long press gestures
//! - Drag gestures
//! - Gesture chaining with `.then()`
//! - Using `on_tap` convenience method

use waterui::app::App;
use waterui::gesture::{DragGesture, LongPressGesture, TapGesture};
use waterui::prelude::*;
use waterui::reactive::Binding;

/// Section displaying tap gesture demos
fn tap_section(tap_count: Binding<i32>) -> impl View {
    vstack((
        text("Tap Gesture").size(20.0),
        "Tap the box below to increment the counter",
        {
            let tap_count = tap_count.clone();
            text("Tap Me!")
                .padding()
                .background(Color::srgb_hex("#2196F3").with_opacity(0.3))
                .gesture(TapGesture::new(), move || {
                    tap_count.set(tap_count.get() + 1);
                })
        },
        hstack(("Tap count: ", waterui::text!("{}", tap_count))),
    ))
    .padding()
}

/// Section displaying double-tap gesture demo
fn double_tap_section(double_tap_count: Binding<i32>) -> impl View {
    vstack((
        text("Double Tap Gesture").size(20.0),
        "Double-tap the box to increment",
        {
            let double_tap_count = double_tap_count.clone();
            text("Double Tap Me!")
                .padding()
                .background(Color::srgb_hex("#4CAF50").with_opacity(0.3))
                .gesture(TapGesture::repeat(2), move || {
                    double_tap_count.set(double_tap_count.get() + 1);
                })
        },
        hstack(("Double tap count: ", waterui::text!("{}", double_tap_count))),
    ))
    .padding()
}

/// Section displaying long press gesture demo
fn long_press_section(long_press_count: Binding<i32>) -> impl View {
    vstack((
        text("Long Press Gesture").size(20.0),
        "Press and hold for 500ms",
        {
            let long_press_count = long_press_count.clone();
            text("Long Press Me!")
                .padding()
                .background(Color::srgb_hex("#FF9800").with_opacity(0.3))
                .gesture(LongPressGesture::new(500), move || {
                    long_press_count.set(long_press_count.get() + 1);
                })
        },
        hstack(("Long press count: ", waterui::text!("{}", long_press_count))),
    ))
    .padding()
}

/// Section displaying drag gesture demo
fn drag_section(drag_count: Binding<i32>) -> impl View {
    vstack((
        text("Drag Gesture").size(20.0),
        "Drag within the box (min 5pt)",
        {
            let drag_count = drag_count.clone();
            text("Drag Here")
                .padding()
                .width(200.0)
                .height(100.0)
                .background(Color::srgb_hex("#9C27B0").with_opacity(0.3))
                .gesture(DragGesture::new(5.0), move || {
                    drag_count.set(drag_count.get() + 1);
                })
        },
        hstack(("Drag events: ", waterui::text!("{}", drag_count))),
    ))
    .padding()
}

/// Section displaying chained gesture demo
fn chained_section(chained_status: Binding<String>) -> impl View {
    vstack((
        text("Chained Gesture").size(20.0),
        "Tap first, then long press to complete",
        {
            let chained_status = chained_status.clone();
            text("Tap then Long Press")
                .padding()
                .background(Color::srgb_hex("#F44336").with_opacity(0.3))
                .gesture(
                    TapGesture::new().then(LongPressGesture::new(300).into()),
                    move || {
                        chained_status.set("Chained gesture completed!".to_string());
                    },
                )
        },
        waterui::text!("{}", chained_status),
    ))
    .padding()
}

/// Section demonstrating on_tap shorthand
fn on_tap_section(tap_count: Binding<i32>) -> impl View {
    vstack((
        text("on_tap Shorthand").size(20.0),
        "Convenient method for simple tap handlers",
        {
            let tap_count = tap_count.clone();
            text("Simple Tap")
                .padding()
                .background(Color::srgb_hex("#00BCD4").with_opacity(0.3))
                .on_tap(move || {
                    tap_count.set(tap_count.get() + 1);
                })
        },
        "This uses the same counter as Section 1",
    ))
    .padding()
}

#[hot_reload]
fn main() -> impl View {
    let tap_count = Binding::int(0);
    let double_tap_count = Binding::int(0);
    let long_press_count = Binding::int(0);
    let drag_count = Binding::int(0);
    let chained_status = Binding::container(String::from("Waiting for tap..."));

    scroll(
        vstack((
            // Header
            text("WaterUI Gesture Examples").size(28.0),
            "Demonstrating gesture recognition and handling",
            Divider,
            spacer(),
            // Gesture sections
            tap_section(tap_count.clone()),
            Divider,
            double_tap_section(double_tap_count),
            Divider,
            long_press_section(long_press_count),
            Divider,
            drag_section(drag_count),
            Divider,
            chained_section(chained_status),
            Divider,
            on_tap_section(tap_count),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
