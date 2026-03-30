//! Hover Example - Demonstrates WaterUI's hover and cursor capabilities
//!
//! This example showcases:
//! - Hover enter/exit events with `on_hover_enter` and `on_hover_exit`
//! - Static cursor styles with `.cursor()`
//! - Reactive cursor styles that change based on state
//! - Lifecycle events with `on_appear` and `on_disappear`

use waterui::animation::{Animation, AnimationExt};
use waterui::app::App;
use waterui::cursor::CursorStyle;
use waterui::graphics::color::Srgb;
use waterui::prelude::*;
use waterui::reactive::{SignalExt, binding};

// Colors for hover states
const HOVER_ACTIVE_COLOR: Srgb = Srgb::from_hex("#4CAF50");
const HOVER_INACTIVE_COLOR: Srgb = Srgb::from_hex("#2196F3");

// Colors for cursor demo boxes
const CURSOR_ARROW_COLOR: Srgb = Srgb::from_hex("#9E9E9E");
const CURSOR_HAND_COLOR: Srgb = Srgb::from_hex("#2196F3");
const CURSOR_TEXT_COLOR: Srgb = Srgb::from_hex("#4CAF50");
const CURSOR_CROSS_COLOR: Srgb = Srgb::from_hex("#FF9800");
const CURSOR_GRAB_COLOR: Srgb = Srgb::from_hex("#9C27B0");
const CURSOR_GRABBING_COLOR: Srgb = Srgb::from_hex("#673AB7");
const CURSOR_NO_COLOR: Srgb = Srgb::from_hex("#F44336");
const CURSOR_WAIT_COLOR: Srgb = Srgb::from_hex("#795548");
const CURSOR_H_RESIZE_COLOR: Srgb = Srgb::from_hex("#00BCD4");
const CURSOR_V_RESIZE_COLOR: Srgb = Srgb::from_hex("#009688");
const CURSOR_MOVE_COLOR: Srgb = Srgb::from_hex("#607D8B");
const CURSOR_COPY_COLOR: Srgb = Srgb::from_hex("#8BC34A");

// Colors for drag states
const DRAG_ACTIVE_COLOR: Srgb = Srgb::from_hex("#FF5722");
const DRAG_INACTIVE_COLOR: Srgb = Srgb::from_hex("#FF9800");

/// Section demonstrating hover enter/exit events
fn hover_events_section(hover_count: &Binding<i32>, is_hovered: &Binding<bool>) -> impl View {
    // Clone for display in text macros
    let hover_count_display = hover_count.clone();
    let is_hovered_display = is_hovered.clone();
    let is_hovered_bg = is_hovered.clone();

    vstack((
        text("Hover Events").headline(),
        "Move your pointer in and out of the box",
        hstack(("Hover events: ", text!("Count: {hover_count_display}"))),
        hstack(("Currently hovered: ", text!("Status: {is_hovered_display}"))),
        text("Hover Me!")
            .padding()
            .width(200.0)
            .height(80.0)
            .background(
                is_hovered_bg
                    .map(|h| {
                        if h {
                            HOVER_ACTIVE_COLOR.with_opacity(0.5)
                        } else {
                            HOVER_INACTIVE_COLOR.with_opacity(0.3)
                        }
                    })
                    .computed(),
            )
            .on_hover_enter(|State(count): State<Binding<i32>>, State(hovered): State<Binding<bool>>| {
                *count.get_mut() += 1;
                hovered.set(true);
            })
            .on_hover_exit(|_: Environment, State(hovered): State<Binding<bool>>| {
                hovered.set(false);
            })
            .state(hover_count)
            .state(is_hovered),
    ))
    .padding()
}

/// Section demonstrating static cursor styles
fn cursor_styles_section() -> impl View {
    vstack((
        text("Cursor Styles").headline(),
        "Hover over each box to see different cursor styles",
        hstack((
            cursor_box("Arrow", CursorStyle::Arrow, CURSOR_ARROW_COLOR),
            cursor_box("Hand", CursorStyle::PointingHand, CURSOR_HAND_COLOR),
            cursor_box("Text", CursorStyle::IBeam, CURSOR_TEXT_COLOR),
            cursor_box("Cross", CursorStyle::Crosshair, CURSOR_CROSS_COLOR),
        )),
        hstack((
            cursor_box("Grab", CursorStyle::OpenHand, CURSOR_GRAB_COLOR),
            cursor_box("Grabbing", CursorStyle::ClosedHand, CURSOR_GRABBING_COLOR),
            cursor_box("No", CursorStyle::NotAllowed, CURSOR_NO_COLOR),
            cursor_box("Wait", CursorStyle::Wait, CURSOR_WAIT_COLOR),
        )),
        hstack((
            cursor_box(
                "H-Resize",
                CursorStyle::ResizeLeftRight,
                CURSOR_H_RESIZE_COLOR,
            ),
            cursor_box("V-Resize", CursorStyle::ResizeUpDown, CURSOR_V_RESIZE_COLOR),
            cursor_box("Move", CursorStyle::Move, CURSOR_MOVE_COLOR),
            cursor_box("Copy", CursorStyle::Copy, CURSOR_COPY_COLOR),
        )),
    ))
    .padding()
}

/// Helper to create a cursor demo box
fn cursor_box(label: &'static str, style: CursorStyle, color: Srgb) -> impl View {
    text(label)
        .caption()
        .padding()
        .background(color.with_opacity(0.3))
        .cursor(style)
}

/// Section demonstrating reactive cursor based on state
fn reactive_cursor_section(is_dragging: &Binding<bool>) -> impl View {
    // Clone for display and reactive properties
    let is_dragging_display = is_dragging.clone();
    let is_dragging_bg = is_dragging.clone();
    let is_dragging_cursor = is_dragging.clone();
    let is_dragging_opacity = is_dragging.clone();

    vstack((
        text("Reactive Cursor").headline(),
        "The cursor changes based on drag state",
        hstack(("State: ", text!("Dragging: {is_dragging_display}"))),
        "(Hover to simulate drag state change)",
        text("Drag Area")
            .padding()
            .width(200.0)
            .height(100.0)
            .background(
                is_dragging_bg
                    .map(|d| {
                        if d {
                            DRAG_ACTIVE_COLOR.with_opacity(0.5)
                        } else {
                            DRAG_INACTIVE_COLOR.with_opacity(0.3)
                        }
                    })
                    .computed(),
            )
            .cursor(
                is_dragging_cursor
                    .map(|d| {
                        if d {
                            CursorStyle::ClosedHand
                        } else {
                            CursorStyle::OpenHand
                        }
                    })
                    .computed(),
            )
            .opacity(
                is_dragging_opacity
                    .select(0.8, 1.0)
                    .with_animation(Animation::default()),
            )
            .on_hover_enter(|State(dragging): State<Binding<bool>>| dragging.set(true))
            .on_hover_exit(|State(dragging): State<Binding<bool>>| dragging.set(false))
            .state(is_dragging),
    ))
    .padding()
}

/// Interactive button showcase
fn interactive_buttons_section() -> impl View {
    vstack((
        text("Interactive Buttons").headline(),
        "Buttons naturally have cursor changes",
        hstack((
            button("Bordered").style(ButtonStyle::Bordered),
            button("Plain").style(ButtonStyle::Plain),
            button("Link Style").style(ButtonStyle::Link),
        )),
        "Link buttons show pointer cursor by default",
    ))
    .padding()
}

fn main() -> impl View {
    let hover_count = binding(0);
    let is_hovered = binding(false);
    let is_dragging = binding(false);

    scroll(
        vstack((
            // Header
            text("WaterUI Hover & Cursor Examples").title(),
            "Demonstrating hover events, cursor styles, and lifecycle hooks",
            Divider,
            spacer(),
            // Sections
            hover_events_section(&hover_count, &is_hovered),
            Divider,
            cursor_styles_section(),
            Divider,
            reactive_cursor_section(&is_dragging),
            Divider,
            interactive_buttons_section(),
            spacer(),
        ))
        .padding(),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
