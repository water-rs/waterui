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
use waterui::graphics::color::Srgb;
use waterui::prelude::*;
use waterui::reactive::Binding;

const TAP_COLOR: Srgb = Srgb::from_hex("#2196F3");
const DOUBLE_TAP_COLOR: Srgb = Srgb::from_hex("#4CAF50");
const LONG_PRESS_COLOR: Srgb = Srgb::from_hex("#FF9800");
const DRAG_COLOR: Srgb = Srgb::from_hex("#9C27B0");
const CHAINED_COLOR: Srgb = Srgb::from_hex("#F44336");
const ON_TAP_COLOR: Srgb = Srgb::from_hex("#00BCD4");

/// Section displaying tap gesture demos
fn tap_section(tap_count: &Binding<i32>) -> impl View {
    vstack((
        text("Tap Gesture").headline(),
        "Tap the box below to increment the counter",
        text!("Tap count: {count}", count = tap_count.clone()),
        text("Tap Me!")
            .padding()
            .background(TAP_COLOR.with_opacity(0.3))
            .gesture(TapGesture::new(), |State(c): State<Binding<i32>>| {
                *c.get_mut() += 1
            })
            .state(tap_count),
    ))
    .padding()
}

/// Section displaying double-tap gesture demo
fn double_tap_section(double_tap_count: &Binding<i32>) -> impl View {
    vstack((
        text("Double Tap Gesture").headline(),
        "Double-tap the box to increment",
        text!(
            "Double tap count: {count}",
            count = double_tap_count.clone()
        ),
        text("Double Tap Me!")
            .padding()
            .background(DOUBLE_TAP_COLOR.with_opacity(0.3))
            .gesture(TapGesture::repeat(2), |State(c): State<Binding<i32>>| {
                *c.get_mut() += 1
            })
            .state(double_tap_count),
    ))
    .padding()
}

/// Section displaying long press gesture demo
fn long_press_section(long_press_count: &Binding<i32>) -> impl View {
    vstack((
        text("Long Press Gesture").headline(),
        "Press and hold for 500ms",
        text!(
            "Long press count: {count}",
            count = long_press_count.clone()
        ),
        text("Long Press Me!")
            .padding()
            .background(LONG_PRESS_COLOR.with_opacity(0.3))
            .gesture(
                LongPressGesture::new(500),
                |State(c): State<Binding<i32>>| *c.get_mut() += 1,
            )
            .state(long_press_count),
    ))
    .padding()
}

/// Section displaying drag gesture demo
fn drag_section(drag_count: &Binding<i32>) -> impl View {
    vstack((
        text("Drag Gesture").headline(),
        "Drag within the box (min 5pt)",
        text!("Drag events: {count}", count = drag_count.clone()),
        text("Drag Here")
            .padding()
            .width(200.0)
            .height(100.0)
            .background(DRAG_COLOR.with_opacity(0.3))
            .gesture(DragGesture::new(5.0), |State(c): State<Binding<i32>>| {
                *c.get_mut() += 1
            })
            .state(drag_count),
    ))
    .padding()
}

/// Section displaying chained gesture demo
fn chained_section(chained_status: &Binding<&'static str>) -> impl View {
    let chained_status_display = chained_status.clone();
    vstack((
        text("Chained Gesture").headline(),
        "Tap first, then long press to complete",
        text!("{chained_status_display}"),
        text("Tap then Long Press")
            .padding()
            .background(CHAINED_COLOR.with_opacity(0.3))
            .gesture(
                TapGesture::new().then(LongPressGesture::new(300)),
                |State(s): State<Binding<&'static str>>| s.set("Chained gesture completed!"),
            )
            .state(chained_status),
    ))
    .padding()
}

/// Section demonstrating on_tap shorthand
fn on_tap_section(tap_count: &Binding<i32>) -> impl View {
    vstack((
        text("on_tap Shorthand").headline(),
        "Convenient method for simple tap handlers",
        "This uses the same counter as Section 1",
        text("Simple Tap")
            .padding()
            .background(ON_TAP_COLOR.with_opacity(0.3))
            .on_tap(|State(c): State<Binding<i32>>| *c.get_mut() += 1)
            .state(tap_count),
    ))
    .padding()
}

fn main() -> impl View {
    let tap_count = Binding::i32(0);
    let double_tap_count = Binding::i32(0);
    let long_press_count = Binding::i32(0);
    let drag_count = Binding::i32(0);
    let chained_status = Binding::container("Waiting for tap...");

    scroll(
        vstack((
            // Header
            text("WaterUI Gesture Examples").title(),
            "Demonstrating gesture recognition and handling",
            Divider,
            spacer(),
            // Gesture sections
            tap_section(&tap_count),
            Divider,
            double_tap_section(&double_tap_count),
            Divider,
            long_press_section(&long_press_count),
            Divider,
            drag_section(&drag_count),
            Divider,
            chained_section(&chained_status),
            Divider,
            on_tap_section(&tap_count),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
