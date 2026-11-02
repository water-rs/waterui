//! The colour a window's committed content lays along its top edge
//! (water-rs/waterui#2727): the topmost single-colour surface spanning the
//! whole edge, composited over the window's clear colour, and the clear
//! colour when nothing spans it.

use kurbo::{Affine, Rect};
use waterui::ViewExt as _;
use waterui::prelude::*;
use waterui_core::AnyView;
use waterui_graphics::draw::WorkingColor;
use waterui_layout::spacer::spacer;
use waterui_layout::stack::vstack;

use super::{MinimalTestTheme, test_environment, test_renderer_with_theme};
use crate::renderer::working_color_to_rgba8;

const WINDOW: Rect = Rect::new(0.0, 0.0, 390.0, 844.0);

fn base() -> WorkingColor {
    WorkingColor::new([0.0, 0.0, 0.0, 1.0])
}

fn top_edge(view: impl waterui_core::View) -> [u8; 4] {
    let env = test_environment();
    let mut renderer = test_renderer_with_theme(MinimalTestTheme::default());
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        AnyView::new(view),
        &env,
        WINDOW,
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();
    renderer.commit_mirror();
    working_color_to_rgba8(renderer.top_edge_color(WINDOW.width(), base()))
}

#[test]
fn a_full_width_bar_at_the_top_sets_the_top_edge_colour() {
    let color = top_edge(vstack((Color::srgb(200, 40, 10).height(60.0), spacer())));
    assert_eq!(color, [200, 40, 10, 255]);
}

#[test]
fn a_translucent_bar_composites_over_what_lies_beneath_it() {
    let color = top_edge(
        vstack((
            Color::srgb(255, 255, 255).with_opacity(0.5).height(60.0),
            spacer(),
        ))
        .background(Color::srgb(0, 0, 255)),
    );
    // Half white over blue, mixed in linear light.
    assert_eq!(color[2], 255);
    assert!(
        (180..=192).contains(&color[0]) && color[0] == color[1],
        "expected linear half-white over blue, got {color:?}"
    );
}

#[test]
fn content_that_does_not_span_the_edge_leaves_the_clear_colour() {
    assert_eq!(top_edge(vstack((spacer(),))), [0, 0, 0, 255]);
    let partial = top_edge(vstack((
        Color::srgb(200, 40, 10).size(100.0, 60.0),
        spacer(),
    )));
    assert_eq!(partial, [0, 0, 0, 255]);
}

#[test]
fn live_colour_changes_the_committed_top_edge_without_recording() {
    let color = Binding::container(Color::srgb(200, 40, 10));
    let env = test_environment();
    let mut renderer = test_renderer_with_theme(MinimalTestTheme::default());
    renderer.reset_scene();
    renderer.begin_rebuild_frame();
    renderer.capture_window_tree(
        AnyView::new(waterui_graphics::color::signal_color(color.clone())),
        &env,
        WINDOW,
        Affine::IDENTITY,
        Affine::IDENTITY,
    );
    renderer.finish_rebuild_frame();
    renderer.commit_mirror();
    let before = working_color_to_rgba8(renderer.top_edge_color(WINDOW.width(), base()));
    color.set(Color::srgb(10, 40, 200));
    let after = working_color_to_rgba8(renderer.top_edge_color(WINDOW.width(), base()));
    assert_eq!(before, [200, 40, 10, 255]);
    assert_eq!(after, [10, 40, 200, 255]);
}
