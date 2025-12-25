//! Hover Example - Demonstrates WaterUI's hover and cursor capabilities
//!
//! This example showcases:
//! - Hover enter/exit events with `on_hover_enter` and `on_hover_exit`
//! - Static cursor styles with `.cursor()`
//! - Reactive cursor styles that change based on state
//! - Lifecycle events with `on_appear` and `on_disappear`

use waterui::animation::AnimationExt;
use waterui::app::App;
use waterui::background::Background;
use waterui::cursor::CursorStyle;
use waterui::prelude::*;
use waterui::reactive::{Binding, SignalExt};
/// Section demonstrating hover enter/exit events
fn hover_events_section(hover_count: Binding<i32>, is_hovered: Binding<bool>) -> impl View {
    let is_hovered_bg = is_hovered.clone();
    let is_hovered_enter = is_hovered.clone();
    let is_hovered_exit = is_hovered.clone();
    let is_hovered_display = is_hovered.clone();
    let hover_count_enter = hover_count.clone();
    let hover_count_display = hover_count.clone();

    vstack((
        text("Hover Events").size(20.0),
        "Move your pointer in and out of the box",
        text("Hover Me!")
            .padding()
            .width(200.0)
            .height(80.0)
            .background(Background::color(
                is_hovered_bg
                    .map(|h| {
                        if h {
                            Color::srgb_hex("#4CAF50").with_opacity(0.5)
                        } else {
                            Color::srgb_hex("#2196F3").with_opacity(0.3)
                        }
                    })
                    .computed(),
            ))
            .on_hover_enter(move || {
                hover_count_enter.set(hover_count_enter.get() + 1);
                is_hovered_enter.set(true);
            })
            .on_hover_exit(move || {
                is_hovered_exit.set(false);
            }),
        hstack(("Hover events: ", text!("Count: {hover_count_display}"))),
        hstack(("Currently hovered: ", text!("Status: {is_hovered_display}"))),
    ))
    .padding()
}

/// Section demonstrating static cursor styles
fn cursor_styles_section() -> impl View {
    vstack((
        text("Cursor Styles").size(20.0),
        "Hover over each box to see different cursor styles",
        hstack((
            cursor_box("Arrow", CursorStyle::Arrow, "#9E9E9E"),
            cursor_box("Hand", CursorStyle::PointingHand, "#2196F3"),
            cursor_box("Text", CursorStyle::IBeam, "#4CAF50"),
            cursor_box("Cross", CursorStyle::Crosshair, "#FF9800"),
        )),
        hstack((
            cursor_box("Grab", CursorStyle::OpenHand, "#9C27B0"),
            cursor_box("Grabbing", CursorStyle::ClosedHand, "#673AB7"),
            cursor_box("No", CursorStyle::NotAllowed, "#F44336"),
            cursor_box("Wait", CursorStyle::Wait, "#795548"),
        )),
        hstack((
            cursor_box("H-Resize", CursorStyle::ResizeLeftRight, "#00BCD4"),
            cursor_box("V-Resize", CursorStyle::ResizeUpDown, "#009688"),
            cursor_box("Move", CursorStyle::Move, "#607D8B"),
            cursor_box("Copy", CursorStyle::Copy, "#8BC34A"),
        )),
    ))
    .padding()
}

/// Helper to create a cursor demo box
fn cursor_box(label: &'static str, style: CursorStyle, color: &'static str) -> impl View {
    text(label)
        .size(12.0)
        .padding_with(EdgeInsets::symmetric(8.0, 4.0))
        .background(Color::srgb_hex(color).with_opacity(0.3))
        .cursor(style)
}

/// Section demonstrating reactive cursor based on state
fn reactive_cursor_section(is_dragging: Binding<bool>) -> impl View {
    let is_dragging_bg = is_dragging.clone();
    let is_dragging_cursor = is_dragging.clone();
    let is_dragging_enter = is_dragging.clone();
    let is_dragging_exit = is_dragging.clone();
    let is_dragging_display = is_dragging.clone();

    vstack((
        text("Reactive Cursor").size(20.0),
        "The cursor changes based on drag state",
        text("Drag Area")
            .padding()
            .width(200.0)
            .height(100.0)
            .background(Background::color(is_dragging_bg.map(|d| {
                if d {
                    Color::srgb_hex("#FF5722").with_opacity(0.5)
                } else {
                    Color::srgb_hex("#FF9800").with_opacity(0.3)
                }
            })))
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
                is_dragging_enter
                    .clone()
                    .animated()
                    .map(|d| if d { 0.8 } else { 1.0 }),
            )
            .on_hover_enter(move || {
                is_dragging_enter.set(true);
            })
            .on_hover_exit(move || {
                is_dragging_exit.set(false);
            }),
        hstack(("State: ", text!("Dragging: {is_dragging_display}"))),
        "(Hover to simulate drag state change)",
    ))
    .padding()
}

/// Interactive button showcase
fn interactive_buttons_section() -> impl View {
    vstack((
        text("Interactive Buttons").size(20.0),
        "Buttons naturally have cursor changes",
        hstack((
            button("Bordered")
                .style(ButtonStyle::Bordered)
                .action(|| {}),
            button("Plain").style(ButtonStyle::Plain).action(|| {}),
            button("Link Style").style(ButtonStyle::Link).action(|| {}),
        )),
        "Link buttons show pointer cursor by default",
    ))
    .padding()
}

#[hot_reload]
fn main() -> impl View {
    let hover_count = Binding::int(0);
    let is_hovered = Binding::bool(false);
    let is_dragging = Binding::bool(false);

    scroll(
        vstack((
            // Header
            text("WaterUI Hover & Cursor Examples").size(28.0),
            "Demonstrating hover events, cursor styles, and lifecycle hooks",
            Divider,
            spacer(),
            // Sections
            hover_events_section(hover_count, is_hovered),
            Divider,
            cursor_styles_section(),
            Divider,
            reactive_cursor_section(is_dragging),
            Divider,
            interactive_buttons_section(),
            spacer(),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
