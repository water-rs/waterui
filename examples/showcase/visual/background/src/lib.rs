//! Background Example - Demonstrates WaterUI's background and cursor capabilities
//!
//! This example showcases:
//! - Hover enter/exit events with `on_hover_enter` and `on_hover_exit`
//! - Static cursor styles with `.cursor()`
//! - Reactive cursor styles that change based on state

use waterui::animation::Animation;
use waterui::app::App;
use waterui::cursor::CursorStyle;
use waterui::graphics::color::Srgb;
use waterui::prelude::*;
use waterui::reactive::{SignalExt, binding};

/// Section demonstrating hover enter/exit events
fn hover_events_section(hover_count: &Binding<i32>, is_hovered: &Binding<bool>) -> impl View {
    const ACTIVE: Srgb = Srgb::from_hex("#4CAF50");
    const INACTIVE: Srgb = Srgb::from_hex("#2196F3");

    let bg = is_hovered.select(ACTIVE.with_opacity(0.5), INACTIVE.with_opacity(0.3));

    vstack((
        text("Hover Events").headline(),
        "Move your pointer in and out of the box",
        hstack(("Hover events: ", text!("Count: {hover_count}"))),
        hstack(("Currently hovered: ", text!("Status: {is_hovered}"))),
        text("Hover Me!")
            .padding()
            .width(200.0)
            .height(80.0)
            .background(bg.computed())
            .on_hover_enter(
                |State(count): State<Binding<i32>>, State(hovered): State<Binding<bool>>| {
                    *count.get_mut() += 1;
                    hovered.set(true);
                },
            )
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
    /// Helper to create a cursor demo box
    fn cursor_box(label: &'static str, style: CursorStyle, color: Srgb) -> impl View {
        text(label)
            .caption()
            .padding()
            .background(color.with_opacity(0.3))
            .cursor(style)
    }

    use CursorStyle::*;

    vstack((
        text("Cursor Styles").headline(),
        "Hover over each box to see different cursor styles",
        hstack((
            cursor_box("Arrow", Arrow, Srgb::from_hex("#9E9E9E")),
            cursor_box("Hand", PointingHand, Srgb::from_hex("#2196F3")),
            cursor_box("Text", IBeam, Srgb::from_hex("#4CAF50")),
            cursor_box("Cross", Crosshair, Srgb::from_hex("#FF9800")),
        )),
        hstack((
            cursor_box("Grab", OpenHand, Srgb::from_hex("#9C27B0")),
            cursor_box("Grabbing", ClosedHand, Srgb::from_hex("#673AB7")),
            cursor_box("No", NotAllowed, Srgb::from_hex("#F44336")),
            cursor_box("Wait", Wait, Srgb::from_hex("#795548")),
        )),
        hstack((
            cursor_box("H-Resize", ResizeLeftRight, Srgb::from_hex("#00BCD4")),
            cursor_box("V-Resize", ResizeUpDown, Srgb::from_hex("#009688")),
            cursor_box("Move", Move, Srgb::from_hex("#607D8B")),
            cursor_box("Copy", Copy, Srgb::from_hex("#8BC34A")),
        )),
    ))
    .padding()
}

/// Section demonstrating reactive cursor based on state
fn reactive_cursor_section(is_dragging: &Binding<bool>) -> impl View {
    const ACTIVE: Srgb = Srgb::from_hex("#FF5722");
    const INACTIVE: Srgb = Srgb::from_hex("#FF9800");

    let bg = is_dragging.select(ACTIVE.with_opacity(0.5), INACTIVE.with_opacity(0.3));
    let cursor = is_dragging.select(CursorStyle::ClosedHand, CursorStyle::OpenHand);
    let opacity = is_dragging.select(0.8, 1.0).with(Animation::default());

    vstack((
        text("Reactive Cursor").headline(),
        "The cursor changes based on drag state",
        hstack(("State: ", text!("Dragging: {d}", d = is_dragging))),
        "(Hover to simulate drag state change)",
        text("Drag Area")
            .padding()
            .width(200.0)
            .height(100.0)
            .background(bg.computed())
            .cursor(cursor.computed())
            .opacity(opacity)
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
            text("WaterUI Background & Cursor Examples").title(),
            "Demonstrating hover events, cursor styles, and reactive backgrounds",
            Divider,
            spacer(),
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
