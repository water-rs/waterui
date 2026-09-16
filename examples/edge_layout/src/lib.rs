//! Edge Layout Example - layout extremes not covered by other examples.
//!
//! Deep stack nesting, a dense grid of non-lazy children, and frame
//! constraint edges (zero, min, max) — the cases that stress measurement
//! recursion and constraint resolution.

use waterui::Identifiable;
use waterui::app::App;
use waterui::layout::stack::{HStack, VStack};
use waterui::prelude::theme_color::MutedForeground;
use waterui::prelude::*;
use waterui::preview;
use waterui::shape::{RoundedRectangle, ShapeExt};

/// 8 levels of alternating hstack/vstack nesting. Recursion through AnyView
/// keeps the concrete type finite while stressing deep layout passes.
fn deep_nest(depth: u32) -> AnyView {
    if depth == 0 {
        return AnyView::new(text("depth 0").caption().foreground(MutedForeground));
    }
    let inner = deep_nest(depth - 1);
    if depth.is_multiple_of(2) {
        AnyView::new(
            hstack((text("·").caption(), inner))
                .spacing(2.0)
                .padding_horizontal(2.0),
        )
    } else {
        AnyView::new(
            vstack((text("·").caption(), inner))
                .spacing(2.0)
                .alignment(HorizontalAlignment::Leading),
        )
    }
}

#[derive(Clone, Copy, Identifiable)]
struct Cell {
    #[id]
    id: u32,
}

/// 16x10 grid of colored squares built eagerly — 160 real views in one pass.
fn dense_grid() -> impl View {
    VStack::for_each(
        (0..16u32).map(|id| Cell { id }).collect::<Vec<_>>(),
        |row| {
            HStack::for_each(
                (0..10u32).map(|id| Cell { id }).collect::<Vec<_>>(),
                move |col| {
                    let hue = (row.id * 10 + col.id) as f32 / 160.0;
                    let (r, g, b) = hsl(hue, 0.7, 0.55);
                    RoundedRectangle::new(0.2)
                        .fill(Color::srgb_f32(r, g, b))
                        .size(28.0, 18.0)
                },
            )
            .spacing(4.0)
        },
    )
    .spacing(4.0)
    .alignment(HorizontalAlignment::Leading)
}

fn hsl(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h * 6.0;
    let x = c * (1.0 - ((hp % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r1 + m, g1 + m, b1 + m)
}

/// Constraint edges: zero-size frame, min forcing growth, max clamping.
fn constraint_edges() -> impl View {
    vstack((
        hstack((
            text("zero:").caption(),
            RoundedRectangle::new(0.5)
                .fill(Color::srgb_hex("#EF4444"))
                .size(0.0, 0.0),
            text("after zero").caption().foreground(MutedForeground),
        ))
        .spacing(8.0),
        hstack((
            text("min 200:").caption(),
            text("x")
                .min_width(200.0)
                .min_height(24.0)
                .background(Color::srgb_hex("#DBEAFE")),
        ))
        .spacing(8.0),
        hstack((
            text("max 60:").caption(),
            text("this label is far too long to fit").max_width(60.0),
        ))
        .spacing(8.0),
    ))
    .alignment(HorizontalAlignment::Leading)
    .spacing(10.0)
}

#[preview]
pub fn demo() -> impl View {
    scroll(
        vstack((
            text("Edge Layout").title(),
            text("Deep nesting, dense children, constraint edges")
                .sub_headline()
                .foreground(MutedForeground),
            Divider,
            text("Deep nesting (8 levels)").sub_headline(),
            deep_nest(8),
            Divider,
            text("Dense grid (160 eager children)").sub_headline(),
            dense_grid(),
            Divider,
            text("Frame constraints").sub_headline(),
            constraint_edges(),
            spacer().min_height(16.0),
        ))
        .alignment(HorizontalAlignment::Leading)
        .padding_with(16.0),
    )
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
