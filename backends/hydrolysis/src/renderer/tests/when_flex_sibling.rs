//! water-rs/hydrolysis#129 — `hstack((column, when(docked, panel)))`: when the
//! `when` child materializes, the enclosing stack must relayout so the flex
//! sibling shrinks to make room. The semantic runtime must agree with the
//! window runner on geometry for the same tree: a column laid out at its
//! width from before the panel materialized overlaps the panel instead of
//! sitting beside it.

use core::time::Duration;
use std::time::Instant;

use accesskit::{Rect, Role, TreeUpdate};
use nami::Binding;
use waterui::ViewExt as _;
use waterui::graphics::color::Srgb;
use waterui::shape::RoundedRectangle;
use waterui::widget::condition::when;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_layout::stack::hstack;
use waterui_shape::ShapeExt as _;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;

const WINDOW_WIDTH: u32 = 1400;
const WINDOW_HEIGHT: u32 = 900;
/// The docked panel the `when` materializes.
const PANEL_WIDTH: f32 = 280.0;

/// Pumps until the runtime reports quiet — never fewer than `min_frames` —
/// then returns the settled merged tree (as the vec `node_bounds` reads).
fn settle(runtime: &mut HeadlessRuntime, at: &mut Instant, min_frames: u32) -> Vec<TreeUpdate> {
    let mut frame = 0;
    loop {
        frame += 1;
        *at += Duration::from_millis(16);
        let _ = runtime.pump_at(false, *at);
        if frame >= min_frames
            && (frame >= 300 || (runtime.is_settled() && !runtime.has_pending_semantic_update()))
        {
            break;
        }
    }
    runtime.accessibility_tree().into_iter().collect()
}

/// The bounds the settled tree carries for the node with `role` and derived
/// `label` — `None` when the node is absent.
fn node_bounds(updates: &[TreeUpdate], role: Role, label: &str) -> Option<Rect> {
    updates.iter().rev().find_map(|update| {
        update.nodes.iter().find_map(|(_, node)| {
            if node.role() == role && node.label() == Some(label) {
                node.bounds()
            } else {
                None
            }
        })
    })
}

/// The materialization that defined the regression: a `set` delivered while
/// the mount build sits inside the rebuild — here an `on_appear` firing from
/// the mount flush — lands after the `when` host already took its initial
/// content. The pending panel must still be applied and the stack must
/// relayout so the flex sibling shrinks beside it; on the issue's rev the
/// swallowed patch mark left the column at full width with the panel never
/// appearing.
#[test]
fn flex_sibling_shrinks_when_when_child_materializes_during_mount() {
    let docked = Binding::container(false);
    let flip = docked.clone();
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || {
            let docked = docked.clone();
            let flip = flip.clone();
            AnyView::new(hstack((
                RoundedRectangle::new(8.0)
                    .fill(Srgb::WHITE)
                    .a11y_label("column")
                    .on_appear(move || flip.set(true)),
                when(docked, || {
                    RoundedRectangle::new(8.0)
                        .fill(Srgb::WHITE)
                        .width(PANEL_WIDTH)
                        .a11y_label("panel")
                }),
            )))
        }),
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    );

    let mut at = Instant::now();
    let updates = settle(&mut runtime, &mut at, 1);

    let column = node_bounds(&updates, Role::Image, "column").expect("column emits");
    let panel = node_bounds(&updates, Role::Image, "panel").expect("panel emits");

    assert!(
        column.x1 <= panel.x0,
        "a materialization delivered mid-mount must still relayout the stack: \
         column {column:?}, panel {panel:?}"
    );
}
